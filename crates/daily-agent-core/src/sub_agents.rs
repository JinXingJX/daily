//! Agents as tools with explicit model bindings, bounded recursive delegation and resumable frames.
use super::*;

/// Meta tool through which the model creates an agent-as-tool.
pub const AGENT_CREATE_TOOL_ID: &str = "agent.create";
pub const AGENT_HANDOFF_TOOL_ID: &str = "agent.handoff";
/// Tool id prefix of each agent-as-tool (`subagent.<id>`).
pub const SUB_AGENT_TOOL_PREFIX: &str = "subagent.";
pub const MAX_SUB_AGENTS: usize = 8;
pub const MAX_SUB_AGENT_INSTRUCTIONS_BYTES: usize = 16 * 1024;
pub(super) const SUB_AGENT_DEFAULT_ITERATIONS: usize = 12;
pub(super) const SUB_AGENT_MAX_ITERATIONS: usize = 40;
pub(super) const SUB_AGENT_OUTPUT_CHARS: usize = 20_000;
pub(super) const SUB_AGENT_FINAL_INSTRUCTIONS: &str = "You are a sub-agent called by another agent. When done, return your result as plain text addressed to that agent: what you found or did, concretely.";

/// An agent the parent model calls like a tool: its own instructions and a subset of the parent's tools.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AgentToolSpec {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The sub-agent's system prompt.
    pub instructions: String,
    /// Ids of parent tools the sub-agent may use.
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub max_iterations: Option<usize>,
}

impl AgentToolSpec {
    pub fn tool_id(&self) -> String {
        format!("{SUB_AGENT_TOOL_PREFIX}{}", self.id)
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SubAgentConfig {
    /// Offers the `agent.create` tool.
    #[serde(default)]
    pub allow_create: bool,
    #[serde(default)]
    pub agents: Vec<AgentToolSpec>,
}

/// A sub-agent run waiting on one of its tools: what the parent called it with and its progress so far.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentFrame {
    #[serde(default)]
    pub handoff: bool,
    #[serde(default)]
    pub completed: Option<AgentTaskOutcome>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub child: Option<Box<SubAgentFrame>>,
    #[serde(default)]
    pub defined_agents: Vec<AgentToolSpec>,
    #[serde(default)]
    pub pending_round: Option<crate::checkpoint::PendingRound>,
    pub agent_id: String,
    pub parent_ply: usize,
    #[serde(default)]
    pub parent_rationale: String,
    #[serde(default)]
    pub parent_input: Value,
    /// The parent's call of this agent, so its answer replays as that call's result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_call: Option<AgentToolCallRecord>,
    #[serde(default)]
    pub observations: Vec<AgentToolObservation>,
    pub next_ply: usize,
}

/// Checks a sub-agent definition against the tools it may use and the agents already defined.
pub fn validate_agent_tool_spec(
    spec: &AgentToolSpec,
    tool_ids: &[String],
    existing: &[AgentToolSpec],
) -> Result<(), String> {
    let id_ok = !spec.id.is_empty()
        && spec.id.len() <= 48
        && spec
            .id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
    if !id_ok {
        return Err("Agent id must be 1-48 characters of letters, digits, _ or -.".into());
    }
    if spec.name.trim().is_empty() || spec.name.len() > 256 {
        return Err("Agent name must be 1-256 bytes.".into());
    }
    if spec.instructions.trim().is_empty() {
        return Err("Agent instructions are required.".into());
    }
    if spec.instructions.len() > MAX_SUB_AGENT_INSTRUCTIONS_BYTES {
        return Err(format!(
            "Agent instructions exceed {MAX_SUB_AGENT_INSTRUCTIONS_BYTES} bytes."
        ));
    }
    if let Some(unknown) = spec.tools.iter().find(|tool| !tool_ids.contains(tool)) {
        return Err(format!(
            "Agent tool {unknown} is not one of the available tools."
        ));
    }
    let replacing = existing.iter().any(|agent| agent.id == spec.id);
    if !replacing && existing.len() >= MAX_SUB_AGENTS {
        return Err(format!("At most {MAX_SUB_AGENTS} agents can be defined."));
    }
    Ok(())
}

/// Adds or replaces `spec` by id.
pub fn upsert_agent_tool_spec(agents: &mut Vec<AgentToolSpec>, spec: AgentToolSpec) {
    match agents.iter_mut().find(|agent| agent.id == spec.id) {
        Some(existing) => *existing = spec,
        None => agents.push(spec),
    }
}

pub(super) fn agent_create_definition(tools: &[AgentTool]) -> ModelToolDefinition {
    let ids = tools
        .iter()
        .map(|tool| tool.id.as_str())
        .collect::<Vec<_>>();
    ModelToolDefinition {
        id: AGENT_CREATE_TOOL_ID.into(),
        name: "Create agent".into(),
        description: format!(
            "Create (or replace, by id) a specialised sub-agent you can then call as tool {SUB_AGENT_TOOL_PREFIX}<id>. It runs on its own with the instructions you write as its system prompt and only the tools you give it, sees the same context blocks as you but not this conversation, and returns a text result. Agents stay available for later messages in this conversation. Use it for focused work worth delegating (drafting, reviewing, research, analysis); at most {MAX_SUB_AGENTS} agents."
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Short id: letters, digits, _ or -." },
                "name": { "type": "string" },
                "description": { "type": "string", "description": "When to call it." },
                "instructions": { "type": "string", "description": "Its system prompt: role, what to do, what to return." },
                "tools": { "type": "array", "items": { "type": "string", "enum": ids }, "description": "Tool ids it may use." },
                "maxIterations": { "type": "number", "description": format!("Model steps it may take (default {SUB_AGENT_DEFAULT_ITERATIONS}, at most {SUB_AGENT_MAX_ITERATIONS}).") }
            },
            "required": ["id", "name", "instructions"]
        }),
    }
}

pub(super) fn define_sub_agent(
    task: &mut AgentTask,
    input: &Value,
) -> Result<AgentToolSpec, String> {
    let spec = serde_json::from_value::<AgentToolSpec>(input.clone())
        .map_err(|error| format!("Invalid agent definition: {error}"))?;
    let spec = AgentToolSpec {
        id: spec.id.trim().to_string(),
        name: spec.name.trim().to_string(),
        ..spec
    };
    let tool_ids = task
        .tools
        .iter()
        .map(|tool| tool.id.clone())
        .collect::<Vec<_>>();
    validate_agent_tool_spec(&spec, &tool_ids, &task.sub_agents.agents)?;
    upsert_agent_tool_spec(&mut task.sub_agents.agents, spec.clone());
    Ok(spec)
}

pub(super) fn sub_agent_for_tool<'a>(
    task: &'a AgentTask,
    tool_id: &str,
) -> Option<&'a AgentToolSpec> {
    let id = tool_id.strip_prefix(SUB_AGENT_TOOL_PREFIX)?;
    task.sub_agents.agents.iter().find(|spec| spec.id == id)
}

#[allow(clippy::large_enum_variant)]
pub(super) enum SubAgentOutcome {
    Completed {
        frame: SubAgentFrame,
        tool_name: String,
        output: String,
        usage: ModelUsage,
        stop_reason: ModelStopReason,
    },
    /// The pause of the sub-agent, with `sub_agent` set to its frame.
    Paused(Box<AgentTaskPause>),
}

pub(super) type DynEmit<'a> = dyn FnMut(AgentTaskEvent) -> Result<(), String> + Send + 'a;
pub(super) type SubAgentFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SubAgentOutcome, AgentError>> + Send + 'a>>;

/// A sub-agent receives its definition's model, allowed tools and explicit delegates.
pub(super) fn sub_agent_task(
    parent: &AgentTask,
    spec: &AgentToolSpec,
    frame: &SubAgentFrame,
) -> Result<AgentTask, AgentError> {
    let definition = parent.agent_definitions.get(&spec.id);
    let model = match definition.and_then(|d| d.model_key.as_deref()) {
        Some(key) => parent.bindings.model(key)?,
        None => parent.model.clone(),
    };
    let mut task = AgentTask::new(
        format!("{}.{}", parent.name, spec.id),
        model,
        frame
            .parent_input
            .get("task")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| frame.parent_input.to_string()),
    );
    task.execution = parent
        .execution
        .child(spec.id.clone(), frame.run_id.clone())?;
    task.system_prompt = spec.instructions.clone();
    task.final_instructions = SUB_AGENT_FINAL_INSTRUCTIONS.into();
    task.context = parent.context.clone();
    task.tools = parent
        .tools
        .iter()
        .filter(|t| spec.tools.contains(&t.id))
        .cloned()
        .collect();
    task.max_iterations = spec
        .max_iterations
        .unwrap_or(SUB_AGENT_DEFAULT_ITERATIONS)
        .clamp(1, SUB_AGENT_MAX_ITERATIONS);
    task.start_ply = frame.next_ply.max(1);
    task.prior_tool_observations = frame.observations.clone();
    task.observation_budget = parent.observation_budget;
    task.max_output_tokens = parent.max_output_tokens;
    task.temperature = parent.temperature;
    task.stream = parent.stream;
    task.bindings = parent.bindings.clone();
    task.agent_definitions = parent.agent_definitions.clone();
    task.resume_sub_agent = frame.child.as_deref().cloned();
    task.pending_round = frame.pending_round.clone();
    if let Some(definition) = definition {
        task.temperature = definition.temperature.unwrap_or(task.temperature);
        task.max_output_tokens = definition
            .max_output_tokens
            .unwrap_or(task.max_output_tokens);
        task.sub_agents.allow_create = definition.allow_create;
        task.allow_handoff = definition.allow_handoff;
        for id in &definition.delegates {
            let delegate = parent.agent_definitions.get(id).ok_or_else(|| {
                AgentError::Output(format!("Delegate definition {id} is unavailable."))
            })?;
            task.sub_agents.agents.push(delegate.spec.clone());
        }
    }
    for spec in &frame.defined_agents {
        upsert_agent_tool_spec(&mut task.sub_agents.agents, spec.clone());
    }
    Ok(task)
}

impl SubAgentFrame {
    pub fn resume_observation(&mut self, observation: AgentToolObservation, next_ply: usize) {
        if let Some(child) = self.child.as_mut() {
            child.resume_observation(observation, next_ply);
        } else {
            self.observations.push(observation);
            self.next_ply = next_ply;
        }
    }
}

/// Runs (or continues) the sub-agent of `frame`, forwarding its events wrapped in `AgentTaskEvent::SubAgent`.
pub(super) fn run_sub_agent<'a>(
    parent: &'a AgentTask,
    mut frame: SubAgentFrame,
    emit: &'a mut DynEmit<'_>,
) -> SubAgentFuture<'a> {
    Box::pin(async move {
        let spec = parent
            .sub_agents
            .agents
            .iter()
            .find(|spec| spec.id == frame.agent_id)
            .cloned()
            .ok_or_else(|| AgentError::Tool {
                tool_id: format!("{SUB_AGENT_TOOL_PREFIX}{}", frame.agent_id),
                message: format!("Agent {} is no longer defined.", frame.agent_id),
            })?;
        if let Some(completed) = frame.completed.take() {
            return Ok(SubAgentOutcome::Completed {
                frame,
                tool_name: spec.name,
                output: completed.final_text,
                usage: ModelUsage::default(),
                stop_reason: completed.stop_reason,
            });
        }
        let child = sub_agent_task(parent, &spec, &frame)?;
        frame.run_id = Some(child.execution.identity.run_id.clone());
        let mut defined_agents = frame.defined_agents.clone();
        let parent_ply = frame.parent_ply;
        let mut forward = |event: AgentTaskEvent| {
            if let AgentTaskEvent::AgentDefined { spec, .. } = &event {
                upsert_agent_tool_spec(&mut defined_agents, spec.clone());
            }
            emit(AgentTaskEvent::SubAgent {
                agent_id: spec.id.clone(),
                agent_name: spec.name.clone(),
                parent_ply,
                event: Box::new(event),
            })
        };
        let forward: &mut DynEmit<'_> = &mut forward;
        let result: Pin<
            Box<dyn Future<Output = Result<AgentTaskRunResult, AgentError>> + Send + '_>,
        > = Box::pin(run_agent_task_until_pause(child, forward));
        let result = result.await?;
        match result {
            AgentTaskRunResult::Completed(outcome) => {
                let mut output = outcome.final_text;
                if !frame.handoff && output.chars().count() > SUB_AGENT_OUTPUT_CHARS {
                    output = output
                        .chars()
                        .take(SUB_AGENT_OUTPUT_CHARS)
                        .collect::<String>()
                        + "…";
                }
                Ok(SubAgentOutcome::Completed {
                    frame,
                    tool_name: spec.name,
                    output,
                    usage: outcome.usage,
                    stop_reason: outcome.stop_reason,
                })
            }
            AgentTaskRunResult::Paused(mut pause) => {
                frame.observations = pause.observations.clone();
                frame.next_ply = pause
                    .sub_agent
                    .as_ref()
                    .map(|f| f.parent_ply)
                    .unwrap_or(pause.ply + 1);
                frame.child = pause.sub_agent.take().map(Box::new);
                frame.defined_agents = defined_agents;
                frame.pending_round = None;
                Ok(SubAgentOutcome::Paused(Box::new(AgentTaskPause {
                    sub_agent: Some(frame),
                    ..pause
                })))
            }
        }
    })
}
