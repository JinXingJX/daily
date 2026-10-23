//! The model/tool loop: requests, tool execution, async pauses, compaction and final output.
use super::*;

#[allow(dead_code)]
pub async fn run_agent_task<F>(
    http: &Client,
    task: AgentTask,
    emit: F,
) -> Result<AgentTaskOutcome, String>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
{
    match run_agent_task_until_pause(http, task, emit).await? {
        AgentTaskRunResult::Completed(outcome) => Ok(outcome),
        AgentTaskRunResult::Paused(pause) => Err(format!(
            "Agent task paused while waiting for async tool {} ({}).",
            pause.tool_name, pause.tool_id
        )),
    }
}

pub async fn run_agent_task_until_pause<F>(
    http: &Client,
    mut task: AgentTask,
    mut emit: F,
) -> Result<AgentTaskRunResult, String>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
{
    let adapter = adapter_for(task.model.provider_kind);
    if !task.inline_images.is_empty() && !adapter.supports_images() {
        return Err(format!(
            "{} does not support image inputs.",
            adapter.kind().label()
        ));
    }

    let mut trace = AgentTaskTrace::default();
    let mut observations = task.prior_tool_observations.clone();
    let mut successful_tool_calls = observations.len();
    let max_iterations = task.max_iterations.max(1);
    let mut start_ply = task.start_ply.clamp(1, max_iterations);

    // A run that paused inside a sub-agent finishes that sub-agent before the parent goes on.
    if let Some(frame) = task.resume_sub_agent.take() {
        let parent_ply = frame.parent_ply;
        match run_sub_agent(http, &task, frame, &mut emit).await {
            Ok(SubAgentOutcome::Completed {
                frame,
                tool_name,
                output,
            }) => {
                successful_tool_calls += 1;
                let tool_id = format!("{SUB_AGENT_TOOL_PREFIX}{}", frame.agent_id);
                let observation = AgentToolObservation::completed(
                    tool_id.clone(),
                    tool_name.clone(),
                    frame.parent_rationale,
                    frame.parent_input,
                    AgentToolResult::with_metadata(output, json!({ "subAgent": frame.agent_id })),
                );
                emit(AgentTaskEvent::ToolResultProduced {
                    ply: parent_ply,
                    tool_id,
                    tool_name,
                    observation: observation.observation.clone(),
                    metadata: observation.metadata.clone(),
                })?;
                observations.push(observation);
            }
            Ok(SubAgentOutcome::Paused(pause)) => {
                return Ok(AgentTaskRunResult::Paused(AgentTaskPause {
                    observations,
                    ..*pause
                }))
            }
            Err(error) => {
                if !task.continue_on_tool_error {
                    return Err(error);
                }
                observations.push(AgentToolObservation::runtime_message(format!(
                    "Sub-agent failed: {error}"
                )));
            }
        }
        start_ply = (parent_ply + 1).clamp(1, max_iterations);
    }

    for ply in start_ply..=max_iterations {
        let force_final = ply == max_iterations;
        if let Some(budget) = task.observation_budget {
            compact_observations(http, &task.model, &mut observations, budget, ply, &mut emit)
                .await?;
        }
        emit(AgentTaskEvent::ModelRequestStarted {
            ply,
            provider_kind: adapter.kind(),
            model: task.model.model.clone(),
        })?;

        let request = ModelCompletionRequest {
            messages: build_task_messages(&task, &observations, force_final, successful_tool_calls),
            temperature: task.temperature,
            max_output_tokens: Some(task.max_output_tokens),
            inline_images: task.inline_images.clone(),
            tools: if force_final {
                Vec::new()
            } else {
                model_tool_definitions(&task)
            },
            session_id: task.session_id,
            turn_id: task.turn_id,
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

        let response = adapter
            .complete(
                http,
                &task.model,
                &request,
                if task.stream && adapter.supports_streaming() {
                    Some(&mut on_chunk)
                } else {
                    None
                },
            )
            .await?;

        emit(AgentTaskEvent::ModelResponseReceived {
            ply,
            raw_output: response.raw_output.clone(),
        })?;
        trace.steps.push(AgentTaskTraceStep::Model {
            ply,
            raw_output: response.raw_output.clone(),
        });

        match model_response_decision(&response) {
            Ok(AgentDecision::Tools(calls)) if !force_final => {
                for request in calls {
                    let AgentToolRequest {
                        rationale,
                        tool_id,
                        input,
                    } = request;
                    if tool_id == AGENT_CREATE_TOOL_ID && task.sub_agents.allow_create {
                        let tool_name = "Create agent".to_string();
                        emit(AgentTaskEvent::ToolSelected {
                            ply,
                            rationale: rationale.clone(),
                            raw_output: response.raw_output.clone(),
                            tool_id: tool_id.clone(),
                            tool_name: tool_name.clone(),
                            input: input.clone(),
                        })?;
                        let (observation, metadata) = match define_sub_agent(&mut task, &input) {
                            Ok(spec) => {
                                successful_tool_calls += 1;
                                let text = format!(
                                    "Agent {} is ready. Call it with tool {} and a task.",
                                    spec.name,
                                    spec.tool_id()
                                );
                                emit(AgentTaskEvent::AgentDefined {
                                    ply,
                                    spec: spec.clone(),
                                })?;
                                (text, json!({ "agent": spec }))
                            }
                            Err(error) => (
                                format!("Tool execution failed: {error}"),
                                json!({ "error": error }),
                            ),
                        };
                        emit(AgentTaskEvent::ToolResultProduced {
                            ply,
                            tool_id: tool_id.clone(),
                            tool_name: tool_name.clone(),
                            observation: observation.clone(),
                            metadata: metadata.clone(),
                        })?;
                        observations.push(AgentToolObservation::completed(
                            tool_id,
                            tool_name,
                            rationale,
                            input,
                            AgentToolResult::with_metadata(observation, metadata),
                        ));
                        continue;
                    }
                    if let Some(spec) = sub_agent_for_tool(&task, &tool_id).cloned() {
                        emit(AgentTaskEvent::ToolSelected {
                            ply,
                            rationale: rationale.clone(),
                            raw_output: response.raw_output.clone(),
                            tool_id: tool_id.clone(),
                            tool_name: spec.name.clone(),
                            input: input.clone(),
                        })?;
                        let frame = SubAgentFrame {
                            agent_id: spec.id.clone(),
                            parent_ply: ply,
                            parent_rationale: rationale.clone(),
                            parent_input: input.clone(),
                            observations: Vec::new(),
                            next_ply: 1,
                        };
                        let (observation, metadata) =
                            match run_sub_agent(http, &task, frame, &mut emit).await {
                                Ok(SubAgentOutcome::Completed { output, .. }) => {
                                    successful_tool_calls += 1;
                                    (output, json!({ "subAgent": spec.id }))
                                }
                                Ok(SubAgentOutcome::Paused(pause)) => {
                                    return Ok(AgentTaskRunResult::Paused(AgentTaskPause {
                                        observations,
                                        trace,
                                        ..*pause
                                    }))
                                }
                                Err(error) => {
                                    if !task.continue_on_tool_error {
                                        return Err(format!("{} failed: {error}", spec.name));
                                    }
                                    (
                                        format!("Tool execution failed: {error}"),
                                        json!({ "error": error, "subAgent": spec.id }),
                                    )
                                }
                            };
                        emit(AgentTaskEvent::ToolResultProduced {
                            ply,
                            tool_id: tool_id.clone(),
                            tool_name: spec.name.clone(),
                            observation: observation.clone(),
                            metadata: metadata.clone(),
                        })?;
                        observations.push(AgentToolObservation::completed(
                            tool_id,
                            spec.name,
                            rationale,
                            input,
                            AgentToolResult::with_metadata(observation, metadata),
                        ));
                        continue;
                    }
                    let Some(tool_index) = task.tools.iter().position(|tool| tool.id == tool_id)
                    else {
                        let message = format!("Model selected unknown tool id {tool_id}.");
                        if task.continue_on_tool_error {
                            observations.push(AgentToolObservation::runtime_message(format!(
                                "Tool error: {message}"
                            )));
                            continue;
                        }
                        return Err(message);
                    };
                    let tool_name = task.tools[tool_index].name.clone();
                    emit(AgentTaskEvent::ToolSelected {
                        ply,
                        rationale: rationale.clone(),
                        raw_output: response.raw_output.clone(),
                        tool_id: tool_id.clone(),
                        tool_name: tool_name.clone(),
                        input: input.clone(),
                    })?;

                    let call = AgentToolCall {
                        tool_id: tool_id.clone(),
                        rationale: rationale.clone(),
                        input: input.clone(),
                    };
                    match task.tools[tool_index].execute(call) {
                        Ok(AgentToolExecution::Completed(result)) => {
                            successful_tool_calls += 1;
                            let observation = AgentToolObservation::completed(
                                tool_id.clone(),
                                tool_name.clone(),
                                rationale.clone(),
                                input.clone(),
                                result,
                            );
                            emit(AgentTaskEvent::ToolResultProduced {
                                ply,
                                tool_id: tool_id.clone(),
                                tool_name: tool_name.clone(),
                                observation: observation.observation.clone(),
                                metadata: observation.metadata.clone(),
                            })?;
                            trace.steps.push(AgentTaskTraceStep::Tool {
                                ply,
                                tool_id: tool_id.clone(),
                                tool_name: tool_name.clone(),
                                input: input.clone(),
                                observation: observation.observation.clone(),
                                metadata: observation.metadata.clone(),
                            });
                            observations.push(observation);
                        }
                        Ok(AgentToolExecution::Pending(pending)) => {
                            emit(AgentTaskEvent::AsyncToolPending {
                                ply,
                                tool_id: tool_id.clone(),
                                tool_name: tool_name.clone(),
                                rationale: rationale.clone(),
                                input: input.clone(),
                                pending_message: pending.pending_message.clone(),
                                metadata: pending.metadata.clone(),
                                observations: observations.clone(),
                            })?;
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
                                trace,
                                sub_agent: None,
                            }));
                        }
                        Err(error) => {
                            let observation = format!("Tool execution failed: {error}");
                            if !task.continue_on_tool_error {
                                return Err(format!("{tool_name} failed: {error}"));
                            }
                            observations.push(AgentToolObservation {
                                tool_id: tool_id.clone(),
                                tool_name: tool_name.clone(),
                                rationale: rationale.clone(),
                                input: input.clone(),
                                pending_message: None,
                                observation: observation.clone(),
                                metadata: json!({ "error": error.clone() }),
                            });
                            emit(AgentTaskEvent::ToolResultProduced {
                                ply,
                                tool_id: tool_id.clone(),
                                tool_name: tool_name.clone(),
                                observation: observation.clone(),
                                metadata: json!({ "error": error }),
                            })?;
                            trace.steps.push(AgentTaskTraceStep::Tool {
                                ply,
                                tool_id,
                                tool_name,
                                input,
                                observation,
                                metadata: json!({ "error": error }),
                            });
                        }
                    }
                }
            }
            Ok(AgentDecision::Tools(calls)) => {
                let tool_ids = calls
                    .into_iter()
                    .map(|call| call.tool_id)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "The model requested tool(s) {tool_ids} after the tool budget was exhausted."
                ));
            }
            Ok(AgentDecision::Final {
                rationale,
                output,
                output_text,
            }) => {
                if task.require_tool_before_final && successful_tool_calls == 0 && !force_final {
                    observations.push(AgentToolObservation::runtime_message(
                        "Final answer rejected by runtime: call at least one tool first."
                            .to_string(),
                    ));
                    continue;
                }
                if task.require_tool_before_final && successful_tool_calls == 0 {
                    return Err(
                        "The model returned final output without calling a required tool.".into(),
                    );
                }
                emit(AgentTaskEvent::FinalSelected {
                    ply,
                    rationale: rationale.clone(),
                    raw_output: response.raw_output,
                    output_text: output_text.clone(),
                    output_value: output.clone(),
                })?;
                trace.steps.push(AgentTaskTraceStep::Final {
                    ply,
                    rationale,
                    output: output.clone(),
                });
                return Ok(AgentTaskRunResult::Completed(AgentTaskOutcome {
                    final_text: output_text,
                    final_value: output,
                    trace,
                }));
            }
            Err(error) => {
                return Err(error);
            }
        }
    }

    Err(format!(
        "Agent task {} exhausted its iteration budget without producing final output.",
        task.name
    ))
}

pub(super) async fn compact_observations<F>(
    http: &Client,
    model: &ModelConfig,
    observations: &mut Vec<AgentToolObservation>,
    budget: ObservationBudget,
    ply: usize,
    emit: &mut F,
) -> Result<(), String>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
{
    let Some(split) = compaction::observation_compaction_split(
        observations,
        budget.max_chars,
        budget.keep_recent,
    ) else {
        return Ok(());
    };
    let history = compaction::render_observations_for_summary(&observations[..split]);
    match compaction::summarize(http, model, None, &history).await {
        Ok(summary) => {
            let summary_chars = summary.chars().count();
            compaction::replace_with_summary(observations, split, summary);
            emit(AgentTaskEvent::ContextCompacted {
                ply,
                folded: split,
                summary_chars,
                error: None,
            })
        }
        Err(error) => {
            compaction::truncate_observations(observations, budget.max_chars, budget.keep_recent);
            emit(AgentTaskEvent::ContextCompacted {
                ply,
                folded: 0,
                summary_chars: 0,
                error: Some(error),
            })
        }
    }
}

pub(super) fn build_task_messages(
    task: &AgentTask,
    observations: &[AgentToolObservation],
    force_final: bool,
    successful_tool_calls: usize,
) -> Vec<ModelMessage> {
    let mut messages = vec![ModelMessage {
        role: ModelMessageRole::System,
        content: format!(
            "{}\n\n{}\n\nTask final instructions:\n{}\n\n{}{}",
            task.system_prompt.trim(),
            SDK_LOOP_PROMPT,
            task.final_instructions.trim(),
            if force_final {
                "Tool budget reached. Produce the final answer now."
            } else {
                "Use a tool call only when a tool is needed; otherwise produce the final answer."
            },
            if task.require_tool_before_final && successful_tool_calls == 0 {
                "\nThis task requires at least one successful tool call before final output."
            } else {
                ""
            }
        ),
    }];

    if !task.context.is_empty() {
        messages.push(ModelMessage {
            role: ModelMessageRole::System,
            content: format_context_blocks(&task.context),
        });
    }

    messages.extend(task.conversation.clone());
    // The run's tool results follow the request they answer, newest last: hoisted into the system
    // prompt, the request stays the latest thing the model reads and it starts that request over.
    let content = if observations.is_empty() {
        task.user_prompt.clone()
    } else {
        format!(
            "{}\n\n{}\n{}",
            task.user_prompt.trim_end(),
            TOOL_PROGRESS_HEADER,
            format_observations(observations)
        )
    };
    messages.push(ModelMessage {
        role: ModelMessageRole::User,
        content,
    });
    messages
}

pub(super) const TOOL_PROGRESS_HEADER: &str = "Progress on this request so far: the tool calls you already made in this run, in order, newest last. They already happened. Continue from the newest one; do not repeat a call that already succeeded.";

pub(super) fn format_context_blocks(context: &[AgentContextBlock]) -> String {
    context
        .iter()
        .map(|block| format!("## {}\n{}", block.title.trim(), block.content.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn format_observations(observations: &[AgentToolObservation]) -> String {
    if observations.is_empty() {
        return "No tool observations yet.".into();
    }
    serde_json::to_string_pretty(
        &observations
            .iter()
            .enumerate()
            .map(|(index, observation)| {
                json!({
                    "type": "tool_message",
                    "toolCallId": format!("tool-call-{}-{}", observation.tool_id, index + 1),
                    "toolId": observation.tool_id,
                    "toolName": observation.tool_name,
                    "rationale": observation.rationale,
                    "input": observation.input,
                    "content": {
                        "status": if observation.pending_message.is_some() { "resumed" } else { "completed" },
                        "initial": observation.pending_message,
                        "result": observation.observation,
                    },
                    "metadata": observation.metadata,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".into())
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
    definitions
}

pub(super) fn model_response_decision(
    response: &crate::model::ModelCompletionResponse,
) -> Result<AgentDecision, String> {
    if !response.tool_calls.is_empty() {
        return Ok(AgentDecision::Tools(
            response
                .tool_calls
                .iter()
                .map(|tool_call| AgentToolRequest {
                    rationale: tool_call
                        .call_id
                        .as_deref()
                        .map(|call_id| format!("Model selected tool call {call_id}."))
                        .unwrap_or_else(|| "Model selected a tool through the model API.".into()),
                    tool_id: tool_call.tool_id.clone(),
                    input: tool_call.arguments.clone(),
                })
                .collect(),
        ));
    }

    let output_text = response.raw_output.trim();
    if output_text.is_empty() {
        return Err("Model returned an empty response.".into());
    }
    Ok(AgentDecision::Final {
        rationale: "Model returned final content without API tool calls.".into(),
        output: final_response_value(output_text),
        output_text: output_text.to_string(),
    })
}

pub(super) fn final_response_value(output_text: &str) -> Value {
    serde_json::from_str::<Value>(output_text)
        .unwrap_or_else(|_| Value::String(output_text.to_string()))
}
