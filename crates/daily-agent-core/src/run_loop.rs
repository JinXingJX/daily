//! The model/tool loop: requests, tool execution, async pauses, compaction and final output.
use super::sub_agents::{
    agent_create_definition, define_sub_agent, run_sub_agent, sub_agent_for_tool, SubAgentOutcome,
};
use super::*;
use crate::checkpoint::{capture, capture_completed, CheckpointPhase, PendingRound, PendingTool};
use crate::model::{ModelCompletionResponse, ModelErrorKind};

/// Runs a task that has no tools which pause. A pause is reported as an error.
pub async fn run_agent_task<F>(task: AgentTask, emit: F) -> Result<AgentTaskOutcome, AgentError>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
{
    match run_agent_task_until_pause(task, emit).await? {
        AgentTaskRunResult::Completed(outcome) => Ok(outcome),
        AgentTaskRunResult::Paused(pause) => Err(AgentError::Output(format!(
            "Agent task paused while waiting for async tool {} ({}).",
            pause.tool_name, pause.tool_id
        ))),
    }
}

fn send<F>(emit: &mut F, event: AgentTaskEvent) -> Result<(), AgentError>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send + ?Sized,
{
    emit(event).map_err(AgentError::Stopped)
}

/// Runs `task` until the model gives its final answer or a tool pauses the run. Every step is
/// reported through `emit`; an error from `emit` stops the run.
pub async fn run_agent_task_until_pause<F>(
    task: AgentTask,
    mut emit: F,
) -> Result<AgentTaskRunResult, AgentError>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
{
    let execution = task.execution.clone();
    execution.begin()?;
    struct EndOnDrop(RunContext, bool);
    impl Drop for EndOnDrop {
        fn drop(&mut self) {
            if !self.1 {
                self.0.cancellation.cancel();
                self.0
                    .finish(RunStatus::Cancelled, Some("Run future dropped.".into()));
            }
        }
    }
    let mut guard = EndOnDrop(execution.clone(), false);
    let result = drive_task(task, |event| {
        if !matches!(event, AgentTaskEvent::SubAgent { .. }) {
            execution.publish(crate::execution::RunEvent::Agent {
                event: Box::new(event.clone()),
            });
        }
        emit(event)
    })
    .await;
    match &result {
        Ok(AgentTaskRunResult::Completed(_)) => execution.finish(RunStatus::Completed, None),
        Ok(AgentTaskRunResult::Paused(_)) => execution.finish(RunStatus::WaitingInput, None),
        Err(error) => execution.finish(
            if matches!(error, AgentError::Stopped(_)) {
                RunStatus::Cancelled
            } else {
                RunStatus::Failed
            },
            Some(error.to_string()),
        ),
    }
    let persisted = execution.persist_latest().await;
    guard.1 = true;
    if result.is_ok() {
        persisted?;
    }
    result
}

async fn drive_task<F>(mut task: AgentTask, mut emit: F) -> Result<AgentTaskRunResult, AgentError>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
{
    task.bindings = task.bindings();
    let tool_ids: Vec<_> = task.tools.iter().map(|t| t.id.clone()).collect();
    if !task.temperature.is_finite() || task.max_output_tokens == 0 {
        return Err(AgentError::Output("Invalid generation parameters.".into()));
    }
    if tool_ids.iter().any(|id| {
        id == AGENT_CREATE_TOOL_ID
            || id == AGENT_HANDOFF_TOOL_ID
            || id.starts_with(SUB_AGENT_TOOL_PREFIX)
    }) {
        return Err(AgentError::Output(
            "Tool ids cannot use reserved agent identifiers.".into(),
        ));
    }
    if tool_ids
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != tool_ids.len()
    {
        return Err(AgentError::Output("Tool ids must be unique.".into()));
    }
    if task.sub_agents.agents.len() > MAX_SUB_AGENTS {
        return Err(AgentError::Output(format!(
            "At most {MAX_SUB_AGENTS} agents can be defined."
        )));
    }
    if task
        .sub_agents
        .agents
        .iter()
        .map(|s| &s.id)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != task.sub_agents.agents.len()
    {
        return Err(AgentError::Output(
            "Agent definition ids must be unique.".into(),
        ));
    }
    for spec in &task.sub_agents.agents {
        validate_agent_tool_spec(spec, &tool_ids, &task.sub_agents.agents)
            .map_err(AgentError::Output)?;
    }
    let adapter = task.model.clone();
    if !task.inline_images.is_empty() && !adapter.supports_images() {
        return Err(AgentError::Model(ModelError::new(
            ModelErrorKind::InvalidRequest,
            format!(
                "{} does not support image inputs.",
                adapter.identity().provider
            ),
        )));
    }

    let mut trace = AgentTaskTrace::default();
    let mut usage = ModelUsage::default();
    let mut observations = task.prior_tool_observations.clone();
    let mut successful_tool_calls = observations
        .iter()
        .filter(|observation| observation.tool_id != RUNTIME_TOOL_ID)
        .count();
    let max_iterations = task.max_iterations.max(1);
    let mut start_ply = task.start_ply.max(1);

    // A run that paused inside a sub-agent finishes that sub-agent before the parent goes on.
    if let Some(frame) = task.resume_sub_agent.take() {
        task.resume_sub_agent = Some(frame.clone());
        capture(
            &task,
            &observations,
            frame.parent_ply,
            CheckpointPhase::Delegating,
            None,
        )
        .await?;
        let parent_ply = frame.parent_ply;
        let handoff = frame.handoff;
        let parent_call = frame.parent_call.clone();
        let tool_id = format!("{SUB_AGENT_TOOL_PREFIX}{}", frame.agent_id);
        match run_sub_agent(&task, frame, &mut emit).await {
            Ok(SubAgentOutcome::Completed {
                frame,
                tool_name,
                output,
                usage: sub_usage,
                stop_reason,
            }) => {
                usage.add(&sub_usage);
                if handoff {
                    return finish_handoff(
                        &mut task,
                        &observations,
                        parent_ply,
                        output,
                        stop_reason,
                        usage,
                        trace,
                        &mut emit,
                    )
                    .await;
                }
                successful_tool_calls += 1;
                let observation = AgentToolObservation::completed(
                    tool_id.clone(),
                    tool_name.clone(),
                    frame.parent_rationale,
                    frame.parent_input,
                    AgentToolResult::with_metadata(output, json!({ "subAgent": frame.agent_id })),
                )
                .with_call(parent_call);
                send(
                    &mut emit,
                    AgentTaskEvent::ToolResultProduced {
                        ply: parent_ply,
                        tool_id,
                        tool_name,
                        observation: observation.observation.clone(),
                        metadata: observation.metadata.clone(),
                        call: observation.call.clone(),
                    },
                )?;
                observations.push(observation);
                task.resume_sub_agent = None;
                if let Some(round) = task.pending_round.as_mut() {
                    round.next_index += 1;
                }
            }
            Ok(SubAgentOutcome::Paused(pause)) => {
                let pause = AgentTaskPause {
                    observations,
                    ..*pause
                };
                capture_pause(&mut task, &pause).await?;
                return Ok(AgentTaskRunResult::Paused(pause));
            }
            Err(error) => {
                if matches!(error, AgentError::Budget(_) | AgentError::Stopped(_)) {
                    return Err(error);
                }
                if !task.continue_on_tool_error {
                    return Err(error);
                }
                observations.push(
                    AgentToolObservation::completed(
                        tool_id.clone(),
                        tool_id,
                        String::new(),
                        Value::Null,
                        AgentToolResult::new(format!("Sub-agent failed: {error}")),
                    )
                    .with_call(parent_call)
                    .failed(),
                );
            }
        }
        task.resume_sub_agent = None;
        start_ply = task
            .pending_round
            .as_ref()
            .map(|r| r.ply)
            .unwrap_or(parent_ply + 1);
        capture(
            &task,
            &observations,
            start_ply,
            CheckpointPhase::Ready,
            None,
        )
        .await?;
    }

    for ply in start_ply..=max_iterations {
        task.execution.check()?;
        capture(&task, &observations, ply, CheckpointPhase::Ready, None).await?;
        let saved_round = task.pending_round.take();
        let next_index = saved_round.as_ref().map(|r| r.next_index).unwrap_or(0);
        let force_final = ply == max_iterations;
        let response = if let Some(round) = saved_round {
            round.response
        } else {
            capture(
                &task,
                &observations,
                ply,
                CheckpointPhase::ModelInFlight,
                None,
            )
            .await?;
            if let Some(budget) = task.observation_budget {
                compact_observations(
                    &task.execution,
                    adapter.as_ref(),
                    &mut observations,
                    budget,
                    ply,
                    &mut emit,
                )
                .await?;
            }
            send(
                &mut emit,
                AgentTaskEvent::ModelRequestStarted {
                    ply,
                    provider: adapter.identity().provider,
                    model: adapter.identity().model,
                },
            )?;

            // On the last step the tools stay declared but may not be called: removing them
            // would change the request prefix, which resets provider caches and invalidates
            // replayed reasoning.
            let request = ModelCompletionRequest {
                purpose: crate::model::ModelRequestPurpose::Generation,
                messages: build_task_messages(&task, &observations, force_final),
                temperature: task.temperature,
                max_output_tokens: Some(task.max_output_tokens),
                tools: model_tool_definitions(&task),
                tool_choice: if force_final {
                    ModelToolChoice::None
                } else {
                    ModelToolChoice::Auto
                },
                request_name: Some(task.name.clone()),
            };

            let mut on_chunk = |chunk: ModelStreamChunk| -> Result<(), String> {
                if chunk.text.is_empty() {
                    return Ok(());
                }
                emit(AgentTaskEvent::ModelStreamChunk {
                    ply,
                    text: chunk.text,
                })
            };
            let response = task
                .execution
                .complete(
                    adapter.as_ref(),
                    &request,
                    if task.stream && adapter.supports_streaming() {
                        Some(&mut on_chunk)
                    } else {
                        None
                    },
                )
                .await?;

            if let Some(reported) = &response.usage {
                usage.add(reported);
            }
            send(
                &mut emit,
                AgentTaskEvent::ModelResponseReceived {
                    ply,
                    raw_output: response.raw_output.clone(),
                    stop_reason: response.stop_reason,
                    usage: response.usage,
                },
            )?;
            trace.steps.push(AgentTaskTraceStep::Model {
                ply,
                raw_output: response.raw_output.clone(),
            });

            response
        };
        task.pending_round = Some(PendingRound {
            ply,
            response: response.clone(),
            next_index,
        });
        capture(&task, &observations, ply, CheckpointPhase::Ready, None).await?;

        if response.tool_calls.is_empty() {
            let output_text = response.raw_output.trim().to_string();
            if output_text.is_empty() {
                return Err(AgentError::Output(
                    "Model returned an empty response.".into(),
                ));
            }
            if task.require_tool_before_final && successful_tool_calls == 0 {
                if force_final {
                    return Err(AgentError::Output(
                        "The model returned final output without calling a required tool.".into(),
                    ));
                }
                task.pending_round = None;
                observations.push(AgentToolObservation::runtime_message(
                    "Final answer rejected by runtime: call at least one tool first.",
                ));
                continue;
            }
            let rationale = "Model returned final content without tool calls.".to_string();
            let output = final_response_value(&output_text);
            task.pending_round = None;
            send(
                &mut emit,
                AgentTaskEvent::FinalSelected {
                    ply,
                    rationale: rationale.clone(),
                    raw_output: response.raw_output,
                    output_text: output_text.clone(),
                    output_value: output.clone(),
                    stop_reason: response.stop_reason,
                },
            )?;
            trace.steps.push(AgentTaskTraceStep::Final {
                ply,
                rationale,
                output: output.clone(),
            });
            let outcome = AgentTaskOutcome {
                final_text: output_text,
                final_value: output,
                stop_reason: response.stop_reason,
                usage,
                trace,
            };
            capture_completed(&task, &observations, ply, &outcome).await?;
            return Ok(AgentTaskRunResult::Completed(outcome));
        }

        if force_final {
            let tool_ids = response
                .tool_calls
                .iter()
                .map(|call| call.tool_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(AgentError::Budget(format!(
                "The model requested tool(s) {tool_ids} after the step budget was exhausted."
            )));
        }

        let records = call_records(ply, &response);
        // The model's own words next to the calls explain them better than anything made up.
        let rationale = response.raw_output.trim().to_string();
        let mut prefetched =
            run_parallel_safe_tools(&task, &response.tool_calls, &rationale, next_index).await;

        for (index, call) in response.tool_calls.iter().enumerate().skip(next_index) {
            let record = records[index].clone();
            let tool_id = call.tool_id.clone();
            let input = call.arguments.clone();
            let tool_name = tool_display_name(&task, &tool_id);
            send(
                &mut emit,
                AgentTaskEvent::ToolSelected {
                    ply,
                    rationale: rationale.clone(),
                    raw_output: response.raw_output.clone(),
                    tool_id: tool_id.clone(),
                    tool_name: tool_name.clone(),
                    input: input.clone(),
                },
            )?;

            // What the call produced: its text, metadata and whether it failed.
            let (text, metadata, failed) =
                if tool_id == AGENT_CREATE_TOOL_ID && task.sub_agents.allow_create {
                    task.execution.reserve_tool().await?;
                    match define_sub_agent(&mut task, &input) {
                        Ok(spec) => {
                            send(
                                &mut emit,
                                AgentTaskEvent::AgentDefined {
                                    ply,
                                    spec: spec.clone(),
                                },
                            )?;
                            (
                                format!(
                                    "Agent {} is ready. Call it with tool {} and a task.",
                                    spec.name,
                                    spec.tool_id()
                                ),
                                json!({ "agent": spec }),
                                false,
                            )
                        }
                        Err(error) => (
                            format!("Tool execution failed: {error}"),
                            json!({ "error": error }),
                            true,
                        ),
                    }
                } else if let Some(spec) = sub_agent_for_tool(&task, &tool_id)
                    .or_else(|| {
                        if task.allow_handoff && tool_id == AGENT_HANDOFF_TOOL_ID {
                            input.get("agent").and_then(Value::as_str).and_then(|id| {
                                task.sub_agents.agents.iter().find(|spec| spec.id == id)
                            })
                        } else {
                            None
                        }
                    })
                    .cloned()
                {
                    task.execution.reserve_tool().await?;
                    let frame = SubAgentFrame {
                        completed: None,
                        handoff: task.allow_handoff && tool_id == AGENT_HANDOFF_TOOL_ID,
                        run_id: Some(task.execution.child(spec.id.clone(), None)?.identity.run_id),
                        child: None,
                        defined_agents: Vec::new(),
                        pending_round: None,
                        agent_id: spec.id.clone(),
                        parent_ply: ply,
                        parent_rationale: rationale.clone(),
                        parent_input: input.clone(),
                        parent_call: Some(record.clone()),
                        observations: Vec::new(),
                        next_ply: 1,
                    };
                    task.resume_sub_agent = Some(frame.clone());
                    capture(&task, &observations, ply, CheckpointPhase::Delegating, None).await?;
                    let child_result = run_sub_agent(&task, frame, &mut emit).await;
                    task.resume_sub_agent = None;
                    match child_result {
                        Ok(SubAgentOutcome::Completed {
                            output,
                            usage: sub_usage,
                            stop_reason,
                            ..
                        }) => {
                            usage.add(&sub_usage);
                            if task.allow_handoff && tool_id == AGENT_HANDOFF_TOOL_ID {
                                return finish_handoff(
                                    &mut task,
                                    &observations,
                                    ply,
                                    output,
                                    stop_reason,
                                    usage,
                                    trace,
                                    &mut emit,
                                )
                                .await;
                            }
                            (output, json!({ "subAgent": spec.id }), false)
                        }
                        Ok(SubAgentOutcome::Paused(mut pause)) => {
                            observations.extend(skipped_observations(
                                &task,
                                &response.tool_calls,
                                &records,
                                index + 1,
                            ));
                            usage.add(&pause.usage);
                            pause.usage = usage;
                            let pause = AgentTaskPause {
                                observations,
                                trace,
                                ..*pause
                            };
                            capture_pause(&mut task, &pause).await?;
                            return Ok(AgentTaskRunResult::Paused(pause));
                        }
                        Err(error) => {
                            if matches!(error, AgentError::Stopped(_) | AgentError::Budget(_)) {
                                return Err(error);
                            }
                            if !task.continue_on_tool_error {
                                return Err(AgentError::Tool {
                                    tool_id,
                                    message: format!("{} failed: {error}", spec.name),
                                });
                            }
                            (
                                format!("Tool execution failed: {error}"),
                                json!({ "error": error.to_string(), "subAgent": spec.id }),
                                true,
                            )
                        }
                    }
                } else if let Some(tool) = task.tools.iter().find(|tool| tool.id == tool_id) {
                    let execution = match prefetched[index].take() {
                        Some(execution) => execution,
                        None => {
                            capture(
                                &task,
                                &observations,
                                ply,
                                CheckpointPhase::ToolInFlight {
                                    call_id: record.call_id.clone(),
                                    replay_safe: tool.parallel_safe,
                                },
                                Some(PendingTool {
                                    ply,
                                    tool_id: tool_id.clone(),
                                    tool_name: tool_name.clone(),
                                    rationale: rationale.clone(),
                                    input: input.clone(),
                                    pending_message: "External tool outcome may be unknown.".into(),
                                    call: Some(record.clone()),
                                }),
                            )
                            .await?;
                            execute_tool(
                                &task.execution,
                                tool,
                                AgentToolCall {
                                    tool_id: tool_id.clone(),
                                    rationale: rationale.clone(),
                                    input: input.clone(),
                                },
                            )
                            .await
                        }
                    };
                    match execution {
                        Ok(AgentToolExecution::Completed(result)) => {
                            (result.observation, result.metadata, false)
                        }
                        Ok(AgentToolExecution::Pending(pending)) => {
                            // The calls after this one are answered as skipped, so the round
                            // stays complete; the model can make them again after the resume.
                            observations.extend(skipped_observations(
                                &task,
                                &response.tool_calls,
                                &records,
                                index + 1,
                            ));
                            task.pending_round = None;
                            capture(
                                &task,
                                &observations,
                                ply + 1,
                                CheckpointPhase::WaitingInput,
                                Some(PendingTool {
                                    ply,
                                    tool_id: tool_id.clone(),
                                    tool_name: tool_name.clone(),
                                    rationale: rationale.clone(),
                                    input: input.clone(),
                                    pending_message: pending.pending_message.clone(),
                                    call: Some(record.clone()),
                                }),
                            )
                            .await?;
                            send(
                                &mut emit,
                                AgentTaskEvent::AsyncToolPending {
                                    ply,
                                    tool_id: tool_id.clone(),
                                    tool_name: tool_name.clone(),
                                    rationale: rationale.clone(),
                                    input: input.clone(),
                                    pending_message: pending.pending_message.clone(),
                                    metadata: pending.metadata.clone(),
                                    observations: observations.clone(),
                                    call: Some(record.clone()),
                                },
                            )?;
                            trace.steps.push(AgentTaskTraceStep::AsyncToolPending {
                                ply,
                                tool_id: tool_id.clone(),
                                tool_name: tool_name.clone(),
                                input: input.clone(),
                                pending_message: pending.pending_message.clone(),
                                metadata: pending.metadata.clone(),
                            });
                            return Ok(AgentTaskRunResult::Paused(AgentTaskPause {
                                ply,
                                tool_id,
                                tool_name,
                                rationale,
                                input,
                                pending_message: pending.pending_message,
                                metadata: pending.metadata,
                                observations,
                                call: Some(record),
                                usage,
                                trace,
                                sub_agent: None,
                            }));
                        }
                        Err(error) => {
                            if matches!(error, AgentError::Stopped(_) | AgentError::Budget(_)) {
                                return Err(error);
                            }
                            (
                                format!("Tool execution failed: {error}"),
                                json!({ "error": error.to_string() }),
                                true,
                            )
                        }
                    }
                } else {
                    // A made-up tool name is the model's mistake to correct, not a reason to
                    // abandon the run.
                    let available = model_tool_definitions(&task)
                        .into_iter()
                        .map(|tool| tool.id)
                        .collect::<Vec<_>>()
                        .join(", ");
                    (
                        format!("Unknown tool {tool_id}. Available tools: {available}."),
                        json!({ "error": "unknown_tool" }),
                        true,
                    )
                };

            if !failed {
                successful_tool_calls += 1;
            }
            let mut observation = AgentToolObservation::completed(
                tool_id.clone(),
                tool_name.clone(),
                rationale.clone(),
                input.clone(),
                AgentToolResult::with_metadata(text, metadata),
            )
            .with_call(Some(record));
            if failed {
                observation = observation.failed();
            }
            observations.push(observation.clone());
            if let Some(round) = task.pending_round.as_mut() {
                round.next_index = index + 1;
            }
            capture(&task, &observations, ply, CheckpointPhase::Ready, None).await?;
            if failed
                && !task.continue_on_tool_error
                && task.tools.iter().any(|tool| tool.id == tool_id)
            {
                return Err(AgentError::Tool {
                    tool_id: tool_id.clone(),
                    message: observation.observation.clone(),
                });
            }
            send(
                &mut emit,
                AgentTaskEvent::ToolResultProduced {
                    ply,
                    tool_id: tool_id.clone(),
                    tool_name: tool_name.clone(),
                    observation: observation.observation.clone(),
                    metadata: observation.metadata.clone(),
                    call: observation.call.clone(),
                },
            )?;
            trace.steps.push(AgentTaskTraceStep::Tool {
                ply,
                tool_id,
                tool_name,
                input,
                observation: observation.observation.clone(),
                metadata: observation.metadata.clone(),
            });
        }
        task.pending_round = None;
        capture(&task, &observations, ply + 1, CheckpointPhase::Ready, None).await?;
    }

    Err(AgentError::Budget(format!(
        "Agent task {} exhausted its step budget without producing final output.",
        task.name
    )))
}

/// One record per call of `response`, with the response's text and opaque items on the first.
fn call_records(ply: usize, response: &ModelCompletionResponse) -> Vec<AgentToolCallRecord> {
    response
        .tool_calls
        .iter()
        .enumerate()
        .map(|(index, call)| AgentToolCallRecord {
            call_id: call
                .call_id
                .clone()
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| format!("call_{ply}_{index}")),
            ply,
            index,
            assistant_text: if index == 0 {
                response.raw_output.clone()
            } else {
                String::new()
            },
            provider: response.provider.clone(),
            provider_items: if index == 0 {
                response.provider_items.clone()
            } else {
                Vec::new()
            },
            provider_meta: call.provider_meta.clone(),
            is_error: false,
        })
        .collect()
}

/// Runs the calls of one response that go to side-effect-free tools at the same time, when
/// there are several. Results are indexed like `calls`; other calls stay `None`.
async fn run_parallel_safe_tools(
    task: &AgentTask,
    calls: &[ModelToolCall],
    rationale: &str,
    from: usize,
) -> Vec<Option<Result<AgentToolExecution, AgentError>>> {
    let mut results = calls.iter().map(|_| None).collect::<Vec<_>>();
    let parallel = calls
        .iter()
        .enumerate()
        .skip(from)
        .filter_map(|(index, call)| {
            // Agent tools take precedence over a plain tool of the same id.
            if sub_agent_for_tool(task, &call.tool_id).is_some() {
                return None;
            }
            task.tools
                .iter()
                .find(|tool| tool.id == call.tool_id && tool.parallel_safe)
                .map(|tool| (index, tool, call))
        })
        .collect::<Vec<_>>();
    if parallel.len() < 2 {
        return results;
    }
    let executions = futures_util::future::join_all(parallel.iter().map(|(_, tool, call)| {
        execute_tool(
            &task.execution,
            tool,
            AgentToolCall {
                tool_id: call.tool_id.clone(),
                rationale: rationale.to_string(),
                input: call.arguments.clone(),
            },
        )
    }))
    .await;
    for ((index, _, _), execution) in parallel.iter().zip(executions) {
        results[*index] = Some(execution);
    }
    results
}

fn tool_display_name(task: &AgentTask, tool_id: &str) -> String {
    if tool_id == AGENT_CREATE_TOOL_ID && task.sub_agents.allow_create {
        return "Create agent".into();
    }
    if let Some(spec) = sub_agent_for_tool(task, tool_id) {
        return spec.name.clone();
    }
    task.tools
        .iter()
        .find(|tool| tool.id == tool_id)
        .map(|tool| tool.name.clone())
        .unwrap_or_else(|| tool_id.to_string())
}

/// Answers for the calls from `from` on that were not run because the round paused.
fn skipped_observations(
    task: &AgentTask,
    calls: &[ModelToolCall],
    records: &[AgentToolCallRecord],
    from: usize,
) -> Vec<AgentToolObservation> {
    calls
        .iter()
        .zip(records)
        .skip(from)
        .map(|(call, record)| {
            AgentToolObservation::completed(
                call.tool_id.clone(),
                tool_display_name(task, &call.tool_id),
                String::new(),
                call.arguments.clone(),
                AgentToolResult::with_metadata(
                    "Not run: the run paused for an earlier tool of the same step. Call this tool again if it is still needed.",
                    json!({ "skipped": true }),
                ),
            )
            .with_call(Some(record.clone()))
            .failed()
        })
        .collect()
}

pub(super) async fn compact_observations<F>(
    execution: &RunContext,
    adapter: &dyn Model,
    observations: &mut Vec<AgentToolObservation>,
    budget: ObservationBudget,
    ply: usize,
    emit: &mut F,
) -> Result<(), AgentError>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
{
    let Some(split) = compaction::observation_compaction_split(
        observations,
        budget.max_tokens,
        budget.keep_recent,
    ) else {
        return Ok(());
    };
    let history = compaction::render_observations_for_summary(&observations[..split]);
    match compaction::summarize_in(execution, adapter, None, &history).await {
        Ok(summary) => {
            let summary_chars = summary.chars().count();
            compaction::replace_with_summary(observations, split, summary);
            send(
                emit,
                AgentTaskEvent::ContextCompacted {
                    ply,
                    folded: split,
                    summary_chars,
                    error: None,
                },
            )
        }
        Err(error) => {
            if matches!(error, AgentError::Stopped(_) | AgentError::Budget(_)) {
                return Err(error);
            }
            compaction::truncate_observations(observations, budget.max_tokens, budget.keep_recent);
            send(
                emit,
                AgentTaskEvent::ContextCompacted {
                    ply,
                    folded: 0,
                    summary_chars: 0,
                    error: Some(error.to_string()),
                },
            )
        }
    }
}

/// The request for the next step: instructions, context, earlier conversation, the user's
/// request, then this run's tool rounds in order. Everything before the newest round is
/// byte-identical to the previous request, which is what provider caches and replayed
/// reasoning rely on; anything that changes per step goes at the end.
pub fn build_task_messages(
    task: &AgentTask,
    observations: &[AgentToolObservation],
    force_final: bool,
) -> Vec<ModelMessage> {
    let mut system = format!(
        "{}\n\n{}\n\nTask final instructions:\n{}",
        task.system_prompt.trim(),
        SDK_LOOP_PROMPT,
        task.final_instructions.trim(),
    );
    if task.require_tool_before_final {
        system.push_str(
            "\nThis task requires at least one successful tool call before the final answer.",
        );
    }
    let mut messages = vec![ModelMessage::system(system.trim_start())];

    if !task.context.is_empty() {
        messages.push(ModelMessage::system(format_context_blocks(&task.context)));
    }
    messages.extend(task.conversation.clone());
    let mut request = ModelMessage::user(task.user_prompt.clone());
    request.images = task.inline_images.clone();
    messages.push(request);
    messages.extend(transcript_messages(observations));
    if force_final {
        messages.push(ModelMessage::user(FORCE_FINAL_NOTE));
    }
    messages
}

/// The tool rounds of a run as model messages: for each model response that called tools, an
/// assistant message with its calls followed by one tool message per result. Observations
/// without a call record (runtime notes, summaries, records of older versions) become notes.
pub fn transcript_messages(observations: &[AgentToolObservation]) -> Vec<ModelMessage> {
    let mut messages = Vec::new();
    let mut index = 0;
    while index < observations.len() {
        let Some(first) = observations[index].call.as_ref() else {
            messages.push(ModelMessage::user(note_text(&observations[index])));
            index += 1;
            continue;
        };
        let ply = first.ply;
        let mut round = observations[index..]
            .iter()
            .map_while(|observation| {
                observation
                    .call
                    .as_ref()
                    .filter(|call| call.ply == ply)
                    .map(|call| (call, observation))
            })
            .collect::<Vec<_>>();
        index += round.len();
        // A resumed tool's result is appended after the results of later calls.
        round.sort_by_key(|(call, _)| call.index);

        let lead = round
            .iter()
            .map(|(call, _)| *call)
            .find(|call| !call.assistant_text.is_empty() || !call.provider_items.is_empty());
        messages.push(ModelMessage {
            role: crate::model::ModelMessageRole::Assistant,
            content: lead
                .map(|call| call.assistant_text.clone())
                .unwrap_or_default(),
            provider: first.provider.clone(),
            provider_items: lead
                .map(|call| call.provider_items.clone())
                .unwrap_or_default(),
            tool_calls: round
                .iter()
                .map(|(call, observation)| ModelToolCall {
                    call_id: Some(call.call_id.clone()),
                    tool_id: observation.tool_id.clone(),
                    arguments: observation.input.clone(),
                    provider_meta: call.provider_meta.clone(),
                })
                .collect(),
            ..ModelMessage::default()
        });
        messages.extend(round.iter().map(|(call, observation)| {
            ModelMessage::tool_result(
                call.call_id.clone(),
                observation.tool_id.clone(),
                observation.observation.clone(),
                call.is_error,
            )
        }));
    }
    messages
}

fn note_text(observation: &AgentToolObservation) -> String {
    if observation.tool_id == RUNTIME_TOOL_ID {
        return format!("[Runtime note]\n{}", observation.observation);
    }
    format!(
        "[Earlier tool call: {} ({})]\ninput: {}\nresult:\n{}",
        observation.tool_name, observation.tool_id, observation.input, observation.observation
    )
}

pub(super) fn format_context_blocks(context: &[AgentContextBlock]) -> String {
    context
        .iter()
        .map(|block| format!("## {}\n{}", block.title.trim(), block.content.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn model_tool_definitions(task: &AgentTask) -> Vec<ModelToolDefinition> {
    let mut definitions = task
        .tools
        .iter()
        .map(|tool| ModelToolDefinition {
            id: tool.id.clone(),
            name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
        })
        .collect::<Vec<_>>();
    if task.sub_agents.allow_create {
        definitions.push(agent_create_definition(&task.tools));
    }
    definitions.extend(task.sub_agents.agents.iter().map(|spec| ModelToolDefinition {
        id: spec.tool_id(),
        name: spec.name.clone(),
        description: format!(
            "Agent-as-tool: {}. Give it a self-contained task; it works with its own tools and returns a text result.",
            if spec.description.trim().is_empty() { spec.name.trim() } else { spec.description.trim() }
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "What the agent should do, with everything it needs to know." }
            },
            "required": ["task"]
        }),
    }));
    if task.allow_handoff && !task.sub_agents.agents.is_empty() {
        definitions.push(ModelToolDefinition { id: AGENT_HANDOFF_TOOL_ID.into(), name: "Transfer to agent".into(), description: "Transfer this task to a declared agent. Its answer becomes the final answer; control does not return to this agent.".into(), input_schema: json!({"type":"object","properties":{"agent":{"type":"string","enum":task.sub_agents.agents.iter().map(|a| a.id.clone()).collect::<Vec<_>>()},"task":{"type":"string"}},"required":["agent","task"]}) });
    }
    definitions
}

pub(super) fn final_response_value(output_text: &str) -> Value {
    serde_json::from_str::<Value>(output_text)
        .unwrap_or_else(|_| Value::String(output_text.to_string()))
}

async fn execute_tool(
    context: &RunContext,
    tool: &AgentTool,
    call: AgentToolCall,
) -> Result<AgentToolExecution, AgentError> {
    context
        .tool(tool.execute(call))
        .await?
        .map_err(|message| AgentError::Tool {
            tool_id: tool.id.clone(),
            message,
        })
}

async fn capture_pause(task: &mut AgentTask, pause: &AgentTaskPause) -> Result<(), AgentError> {
    task.pending_round = None;
    task.resume_sub_agent = pause.sub_agent.clone();
    capture(
        task,
        &pause.observations,
        pause
            .sub_agent
            .as_ref()
            .map(|f| f.parent_ply)
            .unwrap_or(pause.ply + 1),
        CheckpointPhase::WaitingInput,
        Some(PendingTool {
            ply: pause.ply,
            tool_id: pause.tool_id.clone(),
            tool_name: pause.tool_name.clone(),
            rationale: pause.rationale.clone(),
            input: pause.input.clone(),
            pending_message: pause.pending_message.clone(),
            call: pause.call.clone(),
        }),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn finish_handoff<F>(
    task: &mut AgentTask,
    observations: &[AgentToolObservation],
    ply: usize,
    text: String,
    stop_reason: ModelStopReason,
    usage: ModelUsage,
    mut trace: AgentTaskTrace,
    emit: &mut F,
) -> Result<AgentTaskRunResult, AgentError>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
{
    let value = final_response_value(&text);
    trace.steps.push(AgentTaskTraceStep::Final {
        ply,
        rationale: "Final answer from handoff target.".into(),
        output: value.clone(),
    });
    let outcome = AgentTaskOutcome {
        final_text: text.clone(),
        final_value: value.clone(),
        stop_reason,
        usage,
        trace,
    };
    task.resume_sub_agent = None;
    task.pending_round = None;
    capture_completed(task, observations, ply, &outcome).await?;
    send(
        emit,
        AgentTaskEvent::FinalSelected {
            ply,
            rationale: "Final answer from handoff target.".into(),
            raw_output: text.clone(),
            output_text: text,
            output_value: value,
            stop_reason,
        },
    )?;
    Ok(AgentTaskRunResult::Completed(outcome))
}
