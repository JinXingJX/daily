//! Versioned, credential-free execution state with explicit recovery of uncertain operations.
use crate::{definition::*, execution::*, model::*, *};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub const CHECKPOINT_VERSION: u32 = 1;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRound {
    pub ply: usize,
    pub response: ModelCompletionResponse,
    pub next_index: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckpointPhase {
    Ready,
    ModelInFlight,
    ToolInFlight { call_id: String, replay_safe: bool },
    Delegating,
    WaitingInput,
    Completed,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingTool {
    pub ply: usize,
    pub tool_id: String,
    pub tool_name: String,
    pub rationale: String,
    pub input: Value,
    pub pending_message: String,
    pub call: Option<AgentToolCallRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskState {
    pub name: String,
    pub model: ModelIdentity,
    #[serde(default)]
    pub model_binding_key: Option<String>,
    pub models: BTreeMap<String, ModelIdentity>,
    pub system_prompt: String,
    pub user_prompt: String,
    pub final_instructions: String,
    pub context: Vec<AgentContextBlock>,
    pub conversation: Vec<ModelMessage>,
    pub observations: Vec<AgentToolObservation>,
    pub tool_ids: Vec<String>,
    pub max_iterations: usize,
    pub next_ply: usize,
    pub observation_budget: Option<ObservationBudget>,
    pub max_output_tokens: u32,
    pub temperature: f32,
    pub inline_images: Vec<ModelInlineImage>,
    pub stream: bool,
    pub require_tool_before_final: bool,
    pub continue_on_tool_error: bool,
    pub sub_agents: SubAgentConfig,
    #[serde(default)]
    pub allow_handoff: bool,
    pub agent_definitions: BTreeMap<String, AgentDefinition>,
    pub resume_sub_agent: Option<SubAgentFrame>,
    pub pending_round: Option<PendingRound>,
}

impl AgentTaskState {
    pub fn capture(
        task: &AgentTask,
        observations: Vec<AgentToolObservation>,
        next_ply: usize,
    ) -> Self {
        Self {
            name: task.name.clone(),
            model: task.model.identity(),
            model_binding_key: task
                .bindings
                .models
                .iter()
                .find(|(_, model)| Arc::ptr_eq(model, &task.model))
                .map(|(key, _)| key.clone()),
            models: task
                .bindings
                .models
                .iter()
                .map(|(k, v)| (k.clone(), v.identity()))
                .collect(),
            system_prompt: task.system_prompt.clone(),
            user_prompt: task.user_prompt.clone(),
            final_instructions: task.final_instructions.clone(),
            context: task.context.clone(),
            conversation: task.conversation.clone(),
            observations,
            tool_ids: task.tools.iter().map(|t| t.id.clone()).collect(),
            max_iterations: task.max_iterations.max(1),
            next_ply,
            observation_budget: task.observation_budget,
            max_output_tokens: task.max_output_tokens,
            temperature: task.temperature,
            inline_images: task.inline_images.clone(),
            stream: task.stream,
            require_tool_before_final: task.require_tool_before_final,
            continue_on_tool_error: task.continue_on_tool_error,
            sub_agents: task.sub_agents.clone(),
            allow_handoff: task.allow_handoff,
            agent_definitions: task.agent_definitions.clone(),
            resume_sub_agent: task.resume_sub_agent.clone(),
            pending_round: task.pending_round.clone(),
        }
    }
    pub fn bind(
        &self,
        bindings: &Bindings,
        execution: RunContext,
    ) -> Result<AgentTask, AgentError> {
        let model = match &self.model_binding_key {
            Some(key) => bindings.model(key)?,
            None => bindings
                .models
                .values()
                .find(|m| m.identity() == self.model)
                .cloned()
                .ok_or_else(|| {
                    AgentError::Output("Checkpoint model binding is unavailable.".into())
                })?,
        };
        if model.identity() != self.model {
            return Err(AgentError::Output(
                "Checkpoint model identity changed.".into(),
            ));
        }
        for (key, identity) in &self.models {
            if bindings.model(key)?.identity() != *identity {
                return Err(AgentError::Output(format!(
                    "Checkpoint model binding {key} changed identity."
                )));
            }
        }
        Ok(AgentTask {
            name: self.name.clone(),
            model,
            system_prompt: self.system_prompt.clone(),
            user_prompt: self.user_prompt.clone(),
            final_instructions: self.final_instructions.clone(),
            context: self.context.clone(),
            conversation: self.conversation.clone(),
            prior_tool_observations: self.observations.clone(),
            tools: bindings.tools(&self.tool_ids)?,
            max_iterations: self.max_iterations,
            start_ply: self.next_ply,
            observation_budget: self.observation_budget,
            max_output_tokens: self.max_output_tokens,
            temperature: self.temperature,
            inline_images: self.inline_images.clone(),
            stream: self.stream,
            require_tool_before_final: self.require_tool_before_final,
            continue_on_tool_error: self.continue_on_tool_error,
            sub_agents: self.sub_agents.clone(),
            allow_handoff: self.allow_handoff,
            resume_sub_agent: self.resume_sub_agent.clone(),
            agent_definitions: self.agent_definitions.clone(),
            bindings: bindings.clone(),
            execution,
            pending_round: self.pending_round.clone(),
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCheckpoint {
    pub version: u32,
    pub revision: u64,
    #[serde(default)]
    pub sequence: u64,
    pub run: RunIdentity,
    pub limits: RunLimits,
    pub budget: BudgetSnapshot,
    pub phase: CheckpointPhase,
    pub task: AgentTaskState,
    pub pending: Option<PendingTool>,
    #[serde(default)]
    pub outcome: Option<AgentTaskOutcome>,
}

#[derive(Clone)]
pub enum ResumeAction {
    Continue,
    /// Explicitly allow a model request that may already have reached its provider.
    RetryModel,
    /// Only accepted for tools the host marked safe to replay.
    RetrySafeTool,
    ToolResult {
        call_id: String,
        result: AgentToolResult,
        is_error: bool,
    },
}

impl RunCheckpoint {
    pub fn validate(&self) -> Result<(), AgentError> {
        if self.version != CHECKPOINT_VERSION {
            return Err(AgentError::Output(format!(
                "Unsupported checkpoint version {}.",
                self.version
            )));
        }
        self.limits.validate()?;
        if self.run.depth > self.limits.max_depth
            || self.task.next_ply == 0
            || self.task.next_ply > self.task.max_iterations.saturating_add(1)
        {
            return Err(AgentError::Output(
                "Checkpoint contains an invalid execution position.".into(),
            ));
        }
        if let Some(round) = &self.task.pending_round {
            if round.next_index > round.response.tool_calls.len() || round.ply != self.task.next_ply
            {
                return Err(AgentError::Output(
                    "Checkpoint contains an invalid tool cursor.".into(),
                ));
            }
        }
        let mut frame = self.task.resume_sub_agent.as_ref();
        let mut depth = self.run.depth;
        while let Some(current) = frame {
            depth += 1;
            if depth > self.limits.max_depth || current.next_ply == 0 {
                return Err(AgentError::Output(
                    "Checkpoint has an invalid delegation stack.".into(),
                ));
            }
            frame = current.child.as_deref();
        }
        Ok(())
    }
    /// Return a durable completed result without executing any model or tool again.
    pub fn completed_result(
        &self,
        store: Option<Arc<dyn CheckpointStore>>,
    ) -> Result<(RunContext, AgentTaskOutcome), AgentError> {
        self.validate()?;
        if !matches!(self.phase, CheckpointPhase::Completed) {
            return Err(AgentError::Output("This run has not completed.".into()));
        }
        let outcome = self.outcome.clone().ok_or_else(|| {
            AgentError::Output("Completed checkpoint is missing its outcome.".into())
        })?;
        let execution = RunContext::restore(
            self.run.clone(),
            self.limits.clone(),
            self.budget.clone(),
            store,
        );
        execution.seed_checkpoint(self.clone());
        execution.finish(crate::RunStatus::Completed, None);
        Ok((execution, outcome))
    }
    pub fn restore(
        &self,
        bindings: &Bindings,
        action: ResumeAction,
        store: Option<Arc<dyn CheckpointStore>>,
    ) -> Result<AgentTask, AgentError> {
        self.validate()?;
        let mut checkpoint = self.clone();
        match (&self.phase, &action) {
            (CheckpointPhase::Ready, ResumeAction::Continue) => {},
            (CheckpointPhase::ModelInFlight, ResumeAction::RetryModel) => {},
            (CheckpointPhase::Ready, ResumeAction::RetryModel) if self.task.pending_round.as_ref().is_some_and(|r| r.response.tool_calls.is_empty()) => { checkpoint.task.pending_round = None; },
            (CheckpointPhase::ToolInFlight { replay_safe: true, .. }, ResumeAction::RetrySafeTool) => {},
            (CheckpointPhase::ToolInFlight { .. } | CheckpointPhase::WaitingInput, ResumeAction::ToolResult { call_id, result, is_error }) => {
                let pending = self.pending.as_ref().ok_or_else(|| AgentError::Output("Checkpoint is missing its pending tool.".into()))?;
                if pending.call.as_ref().map(|c| c.call_id.as_str()) != Some(call_id.as_str()) { return Err(AgentError::Output("Resume result does not match the pending tool call.".into())); }
                let mut observation = AgentToolObservation::completed(pending.tool_id.clone(), pending.tool_name.clone(), pending.rationale.clone(), pending.input.clone(), result.clone()).with_call(pending.call.clone());
                if *is_error { observation = observation.failed(); }
                if let Some(frame) = checkpoint.task.resume_sub_agent.as_mut() { frame.resume_observation(observation, pending.ply + 1); }
                else { checkpoint.task.observations.push(observation); }
                if matches!(self.phase, CheckpointPhase::ToolInFlight { .. }) {
                    if let Some(round) = checkpoint.task.pending_round.as_mut() { round.next_index += 1; }
                }
            },
            (CheckpointPhase::Completed, _) => return Err(AgentError::Output("A completed run cannot be resumed.".into())),
            _ => return Err(AgentError::Output("Recovery requires an explicit matching model retry or tool result; uncertain operations are never replayed automatically.".into())),
        }
        let execution = RunContext::restore(
            self.run.clone(),
            self.limits.clone(),
            self.budget.clone(),
            store,
        );
        execution.seed_checkpoint(self.clone());
        checkpoint.task.bind(bindings, execution)
    }

    /// Reconcile an interrupted delegation with the latest stored child state before resuming.
    pub async fn restore_tree(
        &self,
        bindings: &Bindings,
        action: ResumeAction,
        store: Arc<dyn CheckpointStore>,
    ) -> Result<AgentTask, AgentError> {
        let mut checkpoint = self.clone();
        merge_budget(
            &mut checkpoint.budget,
            &store.load_tree_budget(&self.run.run_id).await?,
        );
        let mut children = Vec::new();
        hydrate(&mut checkpoint, &action, bindings, &store, &mut children).await?;
        let action = if matches!(self.phase, CheckpointPhase::Delegating) {
            ResumeAction::Continue
        } else {
            action
        };
        let task = checkpoint.restore(bindings, action, Some(store))?;
        for child in children {
            task.execution.seed_checkpoint(child);
        }
        Ok(task)
    }
}

fn hydrate<'a>(
    checkpoint: &'a mut RunCheckpoint,
    action: &'a ResumeAction,
    bindings: &'a Bindings,
    store: &'a Arc<dyn CheckpointStore>,
    children: &'a mut Vec<RunCheckpoint>,
) -> Pin<Box<dyn Future<Output = Result<(), AgentError>> + Send + 'a>> {
    Box::pin(async move {
        checkpoint.validate()?;
        if !matches!(checkpoint.phase, CheckpointPhase::Delegating) {
            return Ok(());
        }
        let frame = checkpoint.task.resume_sub_agent.as_mut().ok_or_else(|| {
            AgentError::Output("Delegation checkpoint is missing its child frame.".into())
        })?;
        let id = frame.run_id.as_ref().ok_or_else(|| {
            AgentError::Output("Delegation checkpoint is missing its child run id.".into())
        })?;
        if let Some(mut child) = store.load(id).await? {
            if child.run.parent_run_id.as_deref() != Some(&checkpoint.run.run_id)
                || child.run.depth != checkpoint.run.depth + 1
            {
                return Err(AgentError::Output(
                    "Child checkpoint belongs to another parent.".into(),
                ));
            }
            hydrate(&mut child, action, bindings, store, children).await?;
            let resolved = if matches!(child.phase, CheckpointPhase::Completed) {
                frame.completed = child.outcome.clone();
                if frame.completed.is_none() {
                    return Err(AgentError::Output(
                        "Completed child is missing its durable outcome.".into(),
                    ));
                }
                None
            } else {
                let child_action = if matches!(child.phase, CheckpointPhase::Ready) {
                    ResumeAction::Continue
                } else {
                    action.clone()
                };
                Some(child.restore(bindings, child_action, None)?)
            };
            if let Some(task) = resolved {
                frame.observations = task.prior_tool_observations;
                frame.next_ply = task.start_ply;
                frame.child = task.resume_sub_agent.map(Box::new);
                frame.pending_round = task.pending_round;
                frame.defined_agents = task.sub_agents.agents;
            }
            merge_budget(&mut checkpoint.budget, &child.budget);
            checkpoint.sequence = checkpoint.sequence.max(child.sequence);
            children.push(child);
        } else if !matches!(action, ResumeAction::Continue) {
            return Err(AgentError::Output(
                "Child has no dispatched operation to resolve.".into(),
            ));
        }
        checkpoint.phase = CheckpointPhase::Ready;
        Ok(())
    })
}

pub(crate) fn merge_budget(target: &mut BudgetSnapshot, source: &BudgetSnapshot) {
    target.model_requests = target.model_requests.max(source.model_requests);
    target.tool_calls = target.tool_calls.max(source.tool_calls);
    target.responses_without_usage = target
        .responses_without_usage
        .max(source.responses_without_usage);
    target.usage.input_tokens = target.usage.input_tokens.max(source.usage.input_tokens);
    target.usage.output_tokens = target.usage.output_tokens.max(source.usage.output_tokens);
    target.usage.cached_input_tokens = target
        .usage
        .cached_input_tokens
        .max(source.usage.cached_input_tokens);
    target.usage.reasoning_tokens = target
        .usage
        .reasoning_tokens
        .max(source.usage.reasoning_tokens);
}

#[async_trait::async_trait]
pub trait CheckpointStore: Send + Sync {
    async fn load(&self, run_id: &str) -> Result<Option<RunCheckpoint>, AgentError>;
    /// Aggregate counter maxima for this run tree; override when sibling runs share budgets.
    async fn load_tree_budget(&self, run_id: &str) -> Result<BudgetSnapshot, AgentError> {
        Ok(self
            .load(run_id)
            .await?
            .map(|c| c.budget)
            .unwrap_or_default())
    }
    /// Compare-and-swap revision fencing; `None` means the run must not exist yet.
    async fn save(
        &self,
        checkpoint: &RunCheckpoint,
        expected_revision: Option<u64>,
    ) -> Result<(), AgentError>;
    async fn load_workflow(
        &self,
        _run_id: &str,
    ) -> Result<Option<crate::workflow::WorkflowState>, AgentError> {
        Err(AgentError::Output(
            "This checkpoint store does not support workflows.".into(),
        ))
    }
    async fn save_workflow(
        &self,
        _state: &crate::workflow::WorkflowState,
        _expected_revision: Option<u64>,
    ) -> Result<(), AgentError> {
        Err(AgentError::Output(
            "This checkpoint store does not support workflows.".into(),
        ))
    }
}

#[derive(Default)]
pub struct MemoryCheckpointStore {
    agents: Mutex<BTreeMap<String, RunCheckpoint>>,
    workflows: Mutex<BTreeMap<String, crate::workflow::WorkflowState>>,
}
#[async_trait::async_trait]
impl CheckpointStore for MemoryCheckpointStore {
    async fn load(&self, run_id: &str) -> Result<Option<RunCheckpoint>, AgentError> {
        Ok(self.agents.lock().unwrap().get(run_id).cloned())
    }
    async fn load_tree_budget(&self, run_id: &str) -> Result<BudgetSnapshot, AgentError> {
        let saved = self.agents.lock().unwrap();
        let mut root = run_id;
        for _ in 0..=32 {
            match saved.get(root).and_then(|c| c.run.parent_run_id.as_deref()) {
                Some(parent) if parent != root => root = parent,
                _ => break,
            }
        }
        let mut ids = std::collections::BTreeSet::from([root]);
        loop {
            let before = ids.len();
            for checkpoint in saved.values() {
                if checkpoint
                    .run
                    .parent_run_id
                    .as_deref()
                    .is_some_and(|p| ids.contains(p))
                {
                    ids.insert(&checkpoint.run.run_id);
                }
            }
            if before == ids.len() {
                break;
            }
        }
        let mut budget = BudgetSnapshot::default();
        for id in ids {
            if let Some(checkpoint) = saved.get(id) {
                merge_budget(&mut budget, &checkpoint.budget);
            }
        }
        Ok(budget)
    }
    async fn save(
        &self,
        checkpoint: &RunCheckpoint,
        expected_revision: Option<u64>,
    ) -> Result<(), AgentError> {
        checkpoint.validate()?;
        let mut saved = self.agents.lock().unwrap();
        if saved.get(&checkpoint.run.run_id).map(|c| c.revision) != expected_revision
            || checkpoint.revision != expected_revision.unwrap_or(0).saturating_add(1)
        {
            return Err(AgentError::Output("Stale checkpoint revision.".into()));
        }
        saved.insert(checkpoint.run.run_id.clone(), checkpoint.clone());
        Ok(())
    }
    async fn load_workflow(
        &self,
        run_id: &str,
    ) -> Result<Option<crate::workflow::WorkflowState>, AgentError> {
        Ok(self.workflows.lock().unwrap().get(run_id).cloned())
    }
    async fn save_workflow(
        &self,
        state: &crate::workflow::WorkflowState,
        expected_revision: Option<u64>,
    ) -> Result<(), AgentError> {
        state.validate()?;
        let mut saved = self.workflows.lock().unwrap();
        if saved.get(&state.run.run_id).map(|s| s.revision) != expected_revision
            || state.revision != expected_revision.unwrap_or(0).saturating_add(1)
        {
            return Err(AgentError::Output(
                "Stale workflow checkpoint revision.".into(),
            ));
        }
        saved.insert(state.run.run_id.clone(), state.clone());
        Ok(())
    }
}

pub(crate) async fn capture(
    task: &AgentTask,
    observations: &[AgentToolObservation],
    next_ply: usize,
    phase: CheckpointPhase,
    pending: Option<PendingTool>,
) -> Result<(), AgentError> {
    task.execution
        .save(RunCheckpoint {
            version: CHECKPOINT_VERSION,
            revision: 0,
            sequence: 0,
            run: task.execution.identity.clone(),
            limits: task.execution.limits().clone(),
            budget: task.execution.budget(),
            phase,
            task: AgentTaskState::capture(task, observations.to_vec(), next_ply),
            pending,
            outcome: None,
        })
        .await
}

pub(crate) async fn capture_completed(
    task: &AgentTask,
    observations: &[AgentToolObservation],
    ply: usize,
    outcome: &AgentTaskOutcome,
) -> Result<(), AgentError> {
    task.execution
        .save(RunCheckpoint {
            version: CHECKPOINT_VERSION,
            revision: 0,
            sequence: 0,
            run: task.execution.identity.clone(),
            limits: task.execution.limits().clone(),
            budget: task.execution.budget(),
            phase: CheckpointPhase::Completed,
            task: AgentTaskState::capture(task, observations.to_vec(), ply),
            pending: None,
            outcome: Some(outcome.clone()),
        })
        .await
}
