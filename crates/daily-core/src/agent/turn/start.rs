use crate::agent;
use crate::agent::sdk::AgentToolObservation;
use crate::agent::turn::hooks::{
    append_hook_error_message, append_hook_report, delay_turn_completion_for_hooks,
    execute_hook_phase, fail_turn_with_message, finalize_turn_after_hooks, hook_enabled,
    set_hook_runtime_state, HookContext,
};
use crate::agent::turn::prompt::{effective_agent_config, effective_model_config};
use crate::agent::types::{AgentTurn, ChatSessionState};
use crate::agent::{
    apply_event_delta, drive_turn, AgentEvent, AgentHookConfig, AgentHookPhase, AgentStore,
    AppSnapshot, DriveTurnResult, ResumeRunInput, RunSessionTurnInput, TurnImageInput,
    TurnRuntimeInput,
};
use crate::limits::{MAX_TURN_IMAGES, MAX_TURN_TEXT_BYTES};
use crate::model::ModelInlineImage;
use crate::runtime::context::RuntimeContext;
use crate::runtime::events::{emit_snapshot, emit_turn_delta};
use crate::state::AppState;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::task::AbortHandle;

/// A chat turn running in the background; `stop` aborts it at its next await point.
pub struct RunningTurn {
    turn_id: u64,
    stopped: Arc<AtomicBool>,
    abort: AbortHandle,
}

pub(crate) const STOPPED_TURN_MESSAGE: &str = "Stopped.";

/// Everything a background turn needs, captured while the state lock is held.
struct TurnPlan {
    session_id: u64,
    turn_id: u64,
    /// The user's request, used for hook prompts.
    request_prompt: String,
    /// The prompt sent to the primary agent (before on-start hook context).
    drive_prompt: String,
    images: Vec<ModelInlineImage>,
    runtime_input: TurnRuntimeInput,
    /// `None` when resuming: on-start hooks already ran for this turn.
    start_hook: Option<AgentHookConfig>,
    finish_hook: AgentHookConfig,
    hooks: HookContext,
}

fn hook_context(
    app_state: &AppState,
    dialogue: Vec<agent::DialogueMessage>,
    memory: Vec<agent::MemoryEntry>,
) -> HookContext {
    HookContext {
        dialogue,
        memory,
        workspace_root: app_state.workspace_root.clone(),
        agent_apps: app_state.agent_apps.clone(),
        manifests: app_state.plugins.installed_manifests(),
        base_agent_config: app_state.agent_config.clone(),
        base_model_config: app_state.model_config.clone(),
    }
}

pub(crate) fn start_session_turn(
    app: &RuntimeContext,
    store: &AgentStore,
    input: RunSessionTurnInput,
) -> Result<AppSnapshot, String> {
    let (request_prompt, display_prompt, images, stored_images) =
        prepare_turn_request(input.text, input.images)?;

    let (snapshot, plan) = {
        let mut app_state = store
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let previous = app_state.clone();
        let turn_id = app_state.queue_turn(input.session_id, &display_prompt, stored_images)?;
        // Every failure after queueing must restore `previous`, or the queued turn would keep
        // the session marked as running.
        let prepared = app_state
            .session(input.session_id)
            .map(|session| (session.messages.clone(), session.memory.clone()))
            .ok_or_else(|| "Session was not found.".to_string())
            .and_then(|(messages, memory)| {
                app_state
                    .find_agent_to_run(input.session_id, &request_prompt)
                    .map(|agent| (messages, memory, agent))
                    .ok_or_else(|| "No agent is available for this session.".to_string())
            });
        let (session_messages, session_memory, bound_agent) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                *app_state = previous;
                return Err(error);
            }
        };
        app_state.assign_turn_agent(input.session_id, turn_id, bound_agent.id, &bound_agent.name);
        let runtime_plugin_manifests = app_state
            .plugins
            .prompt_manifests(&bound_agent.mounted_plugin_ids);
        let runtime_input = TurnRuntimeInput {
            agent: effective_agent_config(
                &app_state.agent_config,
                Some(&bound_agent),
                &runtime_plugin_manifests,
            ),
            model: effective_model_config(&app_state.model_config, Some(&bound_agent)),
            workspace_root: app_state.workspace_root.clone(),
            dialogue: session_messages.clone(),
            memory: session_memory.clone(),
            request_images: images.clone(),
            prior_tool_observations: Vec::new(),
            dynamic_tools: Vec::new(),
            start_ply: 1,
        };
        if let Err(error) = app_state.persist(&store.project_root) {
            *app_state = previous;
            return Err(error);
        }
        let plan = TurnPlan {
            session_id: input.session_id,
            turn_id,
            drive_prompt: request_prompt.clone(),
            request_prompt,
            images,
            runtime_input,
            start_hook: Some(bound_agent.on_start_hook.clone()),
            finish_hook: bound_agent.on_finish_hook.clone(),
            hooks: hook_context(&app_state, session_messages, session_memory),
        };
        (app_state.snapshot(), plan)
    };

    emit_snapshot(app, &snapshot)?;
    spawn_turn(app.clone(), plan);
    Ok(snapshot)
}

pub(crate) fn start_resume_run(
    app: &RuntimeContext,
    store: &AgentStore,
    input: ResumeRunInput,
) -> Result<AppSnapshot, String> {
    let result_text = input.result.trim().to_string();
    if result_text.is_empty() {
        return Err("resume_run requires a non-empty result.".into());
    }

    let (snapshot, plan) = {
        let mut app_state = store
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let previous = app_state.clone();
        app_state.ensure_can_start_turn(input.session_id)?;

        let session = app_state
            .session(input.session_id)
            .cloned()
            .ok_or_else(|| "Session was not found.".to_string())?;
        let turn = session
            .turns
            .iter()
            .find(|turn| turn.id == input.turn_id)
            .cloned()
            .ok_or_else(|| "Turn was not found.".to_string())?;
        if !matches!(turn.status, agent::TurnStatus::WaitingAsyncTool) {
            return Err("This turn is not waiting for an async tool.".into());
        }
        let pending = turn
            .pending_async_tool
            .clone()
            .ok_or_else(|| "This turn is missing async tool resume state.".to_string())?;
        let metadata = json!({
            "ok": input.ok,
            "resume": input.metadata.unwrap_or(Value::Null),
        });
        let observation = AgentToolObservation::resumed(
            pending.tool_id.clone(),
            pending.tool_name.clone(),
            pending.rationale.clone(),
            pending.input.clone(),
            pending.pending_message.clone(),
            if input.ok {
                result_text.clone()
            } else {
                format!("Async tool failed: {result_text}")
            },
            metadata,
        );
        let mut prior_tool_observations = pending.observations.clone();
        prior_tool_observations.push(observation);
        let (runtime_input, agent_app, request_images, hooks) = continuation_input(
            &app_state,
            &session,
            &turn,
            prior_tool_observations,
            pending.ply + 1,
        )?;

        app_state.assign_turn_agent(
            input.session_id,
            input.turn_id,
            agent_app.id,
            &agent_app.name,
        );
        app_state.runtime_status = "Resuming after async tool".into();
        if let Some(session) = app_state.session_mut(input.session_id) {
            session.running_turn_id = Some(input.turn_id);
        }
        if let Some(turn) = app_state.turn_mut(input.session_id, input.turn_id) {
            turn.status = agent::TurnStatus::Running;
            turn.strategy = format!("Resuming after async tool {}.", pending.tool_name);
            turn.pending_async_tool = None;
            turn.partial_response.clear();
        }
        app_state.push_memory(
            input.session_id,
            input.turn_id,
            agent::MemoryKind::ToolResult,
            format!("Async tool completed: {}", pending.tool_name),
            &result_text,
        );
        if let Err(error) = app_state.persist_session(&store.project_root, input.session_id) {
            *app_state = previous;
            return Err(error);
        }
        let plan = TurnPlan {
            session_id: input.session_id,
            turn_id: input.turn_id,
            request_prompt: turn.prompt.clone(),
            drive_prompt: retry_prompt_from_turn(&turn.prompt),
            images: request_images,
            runtime_input,
            start_hook: None,
            finish_hook: agent_app.on_finish_hook.clone(),
            hooks,
        };
        (app_state.snapshot(), plan)
    };

    emit_snapshot(app, &snapshot)?;
    spawn_turn(app.clone(), plan);
    Ok(snapshot)
}

/// Runs on-start hooks, the primary agent, and on-finish hooks in the background,
/// persisting and emitting every turn event as it happens.
fn spawn_turn(app: RuntimeContext, plan: TurnPlan) {
    let session_id = plan.session_id;
    let turn_id = plan.turn_id;
    let stopped = Arc::new(AtomicBool::new(false));
    let task_stopped = stopped.clone();
    let task_app = app.clone();
    // Hold the task table while spawning so the task's own cleanup, which takes the same
    // lock, can only run after its entry exists; otherwise a fast turn could leave a stale entry.
    let store = app.state();
    let mut tasks = store
        .turn_tasks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let handle = tokio::spawn(async move {
        let app = task_app;
        let stopped = task_stopped;
        run_turn(&app, plan, &stopped).await;
        if let Ok(mut tasks) = app.state().turn_tasks.lock() {
            if tasks
                .get(&session_id)
                .is_some_and(|task| task.turn_id == turn_id)
            {
                tasks.remove(&session_id);
            }
        }
    });
    tasks.insert(
        session_id,
        RunningTurn {
            turn_id,
            stopped,
            abort: handle.abort_handle(),
        },
    );
}

async fn run_turn(app: &RuntimeContext, plan: TurnPlan, stopped: &AtomicBool) {
    {
        let TurnPlan {
            session_id,
            turn_id,
            request_prompt,
            mut drive_prompt,
            images,
            runtime_input,
            start_hook,
            finish_hook,
            hooks,
        } = plan;
        let store = app.state();
        let http = store.http.clone();

        if let Some(start_hook) = start_hook.filter(hook_enabled) {
            let _ =
                set_hook_runtime_state(app, &store, session_id, turn_id, "Running on_start hooks.");
            match execute_hook_phase(
                &http,
                &request_prompt,
                &images,
                &hooks,
                &start_hook,
                AgentHookPhase::OnStart,
            )
            .await
            {
                Ok(report) => {
                    if !report.context_blocks.is_empty() {
                        drive_prompt = format!(
                            "{}\n\nOn-start hook context:\n{}",
                            request_prompt,
                            report.context_blocks.join("\n\n")
                        );
                    }
                    let _ = append_hook_report(app, &store, session_id, turn_id, &report);
                }
                Err(error) => {
                    let _ = fail_turn_with_message(app, &store, session_id, turn_id, error);
                    return;
                }
            }
        }

        let finish_hook_enabled = hook_enabled(&finish_hook);
        let result = drive_turn(
            &http,
            session_id,
            turn_id,
            &drive_prompt,
            runtime_input,
            |event| {
                let delta = {
                    let mut state = match store.state.lock() {
                        Ok(state) => state,
                        Err(_) => return Err("Application state is unavailable.".into()),
                    };
                    // A stopped turn was already recorded; drop events still in flight.
                    if stopped.load(Ordering::SeqCst) {
                        return Err(STOPPED_TURN_MESSAGE.into());
                    }
                    let should_persist = !matches!(&event, AgentEvent::ModelStreamChunk { .. });
                    let delta = match event {
                        AgentEvent::TurnCompleted {
                            session_id,
                            turn_id,
                            strategy,
                        } if finish_hook_enabled => Some(delay_turn_completion_for_hooks(
                            &mut state, session_id, turn_id, strategy,
                        )),
                        other => apply_event_delta(&mut state, other),
                    };
                    if should_persist {
                        state.persist_session(&store.project_root, session_id)?;
                    }
                    delta
                };
                if let Some(delta) = delta {
                    emit_turn_delta(app, delta)?;
                }
                Ok(())
            },
        )
        .await;
        if stopped.load(Ordering::SeqCst) {
            return;
        }

        let drive_result = match result {
            Ok(drive_result) => drive_result,
            Err(error) => {
                let _ = fail_turn_with_message(app, &store, session_id, turn_id, error);
                return;
            }
        };

        if !matches!(drive_result, DriveTurnResult::Completed) || !finish_hook_enabled {
            return;
        }

        let main_output = {
            let state = match store.state.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            state
                .session(session_id)
                .and_then(|session| session.turns.iter().find(|turn| turn.id == turn_id))
                .map(|turn| turn.response.clone())
                .unwrap_or_default()
        };

        let _ =
            set_hook_runtime_state(app, &store, session_id, turn_id, "Running on_finish hooks.");

        let finish_prompt = format!(
            "Original user request:\n{}\n\nPrimary agent output:\n{}",
            request_prompt.trim(),
            main_output.trim()
        );

        match execute_hook_phase(
            &http,
            &finish_prompt,
            &images,
            &hooks,
            &finish_hook,
            AgentHookPhase::OnFinish,
        )
        .await
        {
            Ok(report) => {
                let _ = append_hook_report(app, &store, session_id, turn_id, &report);
            }
            Err(error) => {
                let _ = append_hook_error_message(
                    app,
                    &store,
                    session_id,
                    turn_id,
                    format!("On-finish hooks failed: {error}"),
                );
            }
        }
        let _ = finalize_turn_after_hooks(app, &store, session_id, turn_id);
    }
}

/// The runtime input for continuing `turn` with the agent it ran with.
fn continuation_input(
    app_state: &AppState,
    session: &ChatSessionState,
    turn: &AgentTurn,
    prior_tool_observations: Vec<AgentToolObservation>,
    start_ply: usize,
) -> Result<
    (
        TurnRuntimeInput,
        agent::AgentAppConfig,
        Vec<ModelInlineImage>,
        HookContext,
    ),
    String,
> {
    let agent_app = turn
        .agent_app_id
        .and_then(|agent_id| {
            app_state
                .agent_apps
                .iter()
                .find(|agent| agent.id == agent_id)
                .cloned()
        })
        .or_else(|| app_state.find_agent_to_run(session.id, &turn.prompt))
        .ok_or_else(|| "No agent is available to continue this turn.".to_string())?;
    let manifests = app_state
        .plugins
        .prompt_manifests(&agent_app.mounted_plugin_ids);
    let request_images = turn
        .images
        .clone()
        .into_iter()
        .map(|image| image.into_model_image())
        .collect::<Result<Vec<_>, String>>()?;
    let hooks = hook_context(app_state, session.messages.clone(), session.memory.clone());
    let runtime_input = TurnRuntimeInput {
        agent: effective_agent_config(&app_state.agent_config, Some(&agent_app), &manifests),
        model: effective_model_config(&app_state.model_config, Some(&agent_app)),
        workspace_root: app_state.workspace_root.clone(),
        dialogue: hooks.dialogue.clone(),
        memory: hooks.memory.clone(),
        request_images: request_images.clone(),
        prior_tool_observations,
        dynamic_tools: Vec::new(),
        start_ply,
    };
    Ok((runtime_input, agent_app, request_images, hooks))
}

/// Continues the session's last turn after it failed or was stopped: same turn id, no new
/// user message, starting from the ply after the last tool result with those results kept.
pub(crate) fn start_continue_turn(
    app: &RuntimeContext,
    store: &AgentStore,
    session_id: u64,
    turn_id: u64,
) -> Result<AppSnapshot, String> {
    let (snapshot, plan) = {
        let mut app_state = store
            .state
            .lock()
            .map_err(|_| "Application state is unavailable.".to_string())?;
        let previous = app_state.clone();
        app_state
            .continue_failed_turn(session_id, turn_id)
            .inspect_err(|_| *app_state = previous.clone())?;
        let session = app_state
            .session(session_id)
            .cloned()
            .ok_or_else(|| "Session was not found.".to_string())?;
        let turn = session
            .turns
            .iter()
            .find(|turn| turn.id == turn_id)
            .cloned()
            .ok_or_else(|| "Turn was not found.".to_string())?;
        let checkpoint = turn.checkpoint.clone().unwrap_or(agent::TurnCheckpoint {
            observations: Vec::new(),
            next_ply: 1,
        });
        let (runtime_input, agent_app, request_images, hooks) = match continuation_input(
            &app_state,
            &session,
            &turn,
            checkpoint.observations,
            checkpoint.next_ply,
        ) {
            Ok(input) => input,
            Err(error) => {
                *app_state = previous;
                return Err(error);
            }
        };
        if let Err(error) = app_state.persist_session(&store.project_root, session_id) {
            *app_state = previous;
            return Err(error);
        }
        let plan = TurnPlan {
            session_id,
            turn_id,
            request_prompt: turn.prompt.clone(),
            drive_prompt: retry_prompt_from_turn(&turn.prompt),
            images: request_images,
            runtime_input,
            start_hook: None,
            finish_hook: agent_app.on_finish_hook.clone(),
            hooks,
        };
        (app_state.snapshot(), plan)
    };

    emit_snapshot(app, &snapshot)?;
    spawn_turn(app.clone(), plan);
    Ok(snapshot)
}

/// Stops the session's running turn. It is recorded as failed with its checkpoint, so
/// `start_continue_turn` can pick it up again.
pub(crate) fn stop_session_turn(
    app: &RuntimeContext,
    store: &AgentStore,
    session_id: u64,
) -> Result<(), String> {
    let task = store
        .turn_tasks
        .lock()
        .map_err(|_| "Turn tasks are unavailable.".to_string())?
        .remove(&session_id);
    let Some(task) = task else {
        return Err("This session has no running turn.".into());
    };
    task.stopped.store(true, Ordering::SeqCst);
    task.abort.abort();
    let running = store
        .state
        .lock()
        .map_err(|_| "Application state is unavailable.".to_string())?
        .session(session_id)
        .and_then(|session| session.turns.iter().find(|turn| turn.id == task.turn_id))
        .is_some_and(|turn| {
            matches!(
                turn.status,
                agent::TurnStatus::Queued | agent::TurnStatus::Running
            )
        });
    if running {
        fail_turn_with_message(
            app,
            store,
            session_id,
            task.turn_id,
            STOPPED_TURN_MESSAGE.into(),
        )?;
    }
    Ok(())
}

pub(crate) fn prepare_turn_request(
    text: String,
    images: Vec<TurnImageInput>,
) -> Result<(String, String, Vec<ModelInlineImage>, Vec<TurnImageInput>), String> {
    if text.len() > MAX_TURN_TEXT_BYTES {
        return Err(format!(
            "Message exceeds the {MAX_TURN_TEXT_BYTES} byte limit."
        ));
    }
    if images.len() > MAX_TURN_IMAGES {
        return Err(format!(
            "A turn can include at most {MAX_TURN_IMAGES} images."
        ));
    }
    let stored_images = images.clone();
    let images = images
        .into_iter()
        .map(|image| image.into_model_image())
        .collect::<Result<Vec<_>, String>>()?;

    let prompt = text.trim().to_string();
    if prompt.is_empty() && images.is_empty() {
        return Err("Message or image input is required.".into());
    }

    let prompt = if prompt.is_empty() {
        "Analyze the attached image(s).".to_string()
    } else {
        prompt
    };
    let display_prompt = if images.is_empty() {
        prompt.clone()
    } else {
        let image_labels = images
            .iter()
            .map(|image| {
                image
                    .name
                    .clone()
                    .unwrap_or_else(|| image.mime_type.clone())
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("{prompt}\n\n[Attached images: {image_labels}]")
    };

    Ok((prompt, display_prompt, images, stored_images))
}

pub(crate) fn retry_prompt_from_turn(prompt: &str) -> String {
    const ATTACHED_IMAGES_SUFFIX: &str = "\n\n[Attached images: ";

    if let Some(index) = prompt.rfind(ATTACHED_IMAGES_SUFFIX) {
        if prompt.ends_with(']') {
            let stripped = prompt[..index].trim_end();
            if !stripped.is_empty() {
                return stripped.to_string();
            }
        }
    }

    prompt.to_string()
}
