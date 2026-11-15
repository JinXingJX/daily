//! Bounded serial, parallel and dependency-ordered multi-agent workflows.
use crate::{checkpoint::RunCheckpoint, definition::*, execution::*, *};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowNode {
    pub id: String,
    pub agent_id: String,
    pub prompt: String,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    #[default]
    FailFast,
    Collect,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub agents: BTreeMap<String, AgentDefinition>,
    pub nodes: Vec<WorkflowNode>,
    pub failure_policy: FailurePolicy,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum NodeResult {
    Running {
        run_id: String,
    },
    Completed {
        output: Value,
        text: String,
    },
    WaitingInput {
        checkpoint: Box<RunCheckpoint>,
    },
    Failed {
        error: String,
        checkpoint: Option<Box<RunCheckpoint>>,
    },
    Blocked {
        dependency: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowState {
    #[serde(default)]
    pub sequence: u64,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub error: Option<String>,
    pub plan: Workflow,
    pub version: u32,
    pub run: RunIdentity,
    pub limits: RunLimits,
    pub budget: BudgetSnapshot,
    pub results: BTreeMap<String, NodeResult>,
}

impl Workflow {
    pub fn validate(&self) -> Result<(), AgentError> {
        if self.nodes.is_empty() || self.nodes.len() > 256 {
            return Err(AgentError::Output("Workflows require 1-256 nodes.".into()));
        }
        let ids: BTreeSet<_> = self.nodes.iter().map(|n| &n.id).collect();
        if ids.len() != self.nodes.len() || ids.iter().any(|id| id.is_empty()) {
            return Err(AgentError::Output(
                "Workflow node ids must be non-empty and unique.".into(),
            ));
        }
        for node in &self.nodes {
            if !self.agents.contains_key(&node.agent_id)
                || node
                    .dependencies
                    .iter()
                    .any(|id| !ids.contains(id) || id == &node.id)
            {
                return Err(AgentError::Output(format!(
                    "Invalid workflow node {}.",
                    node.id
                )));
            }
        }
        let mut visited = BTreeSet::new();
        loop {
            let before = visited.len();
            for node in &self.nodes {
                if node.dependencies.iter().all(|id| visited.contains(id)) {
                    visited.insert(node.id.clone());
                }
            }
            if visited.len() == self.nodes.len() {
                return Ok(());
            }
            if visited.len() == before {
                return Err(AgentError::Output(
                    "Workflow dependency cycle detected.".into(),
                ));
            }
        }
    }
    pub fn serial(agents: BTreeMap<String, AgentDefinition>, mut nodes: Vec<WorkflowNode>) -> Self {
        for index in 1..nodes.len() {
            nodes[index].dependencies = vec![nodes[index - 1].id.clone()];
        }
        Self {
            agents,
            nodes,
            failure_policy: FailurePolicy::FailFast,
        }
    }
    pub fn parallel(
        agents: BTreeMap<String, AgentDefinition>,
        mut nodes: Vec<WorkflowNode>,
    ) -> Self {
        for node in &mut nodes {
            node.dependencies.clear();
        }
        Self {
            agents,
            nodes,
            failure_policy: FailurePolicy::Collect,
        }
    }
    /// Resume results are supplied explicitly; waiting/failed nodes must be resolved by the caller.
    /// Futures are owned by this call, so dropping it drops every active node.
    pub async fn run(
        &self,
        bindings: &Bindings,
        execution: RunContext,
        previous: Option<WorkflowState>,
    ) -> Result<WorkflowState, AgentError> {
        self.validate()?;
        execution.begin()?;
        struct CancelOnDrop(RunContext);
        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                if self.0.status() == RunStatus::Running {
                    self.0.cancellation.cancel();
                    self.0.finish(
                        RunStatus::Cancelled,
                        Some("Workflow future dropped.".into()),
                    );
                }
            }
        }
        let _guard = CancelOnDrop(execution.clone());
        let mut bindings = bindings.clone();
        bindings.agents.extend(self.agents.clone());
        for node in &self.nodes {
            self.agents[&node.agent_id].instantiate(&bindings, node.prompt.clone())?;
        }
        let mut state = match previous {
            Some(state)
                if state.version == 1
                    && state.plan == *self
                    && state.run.run_id == execution.identity.run_id
                    && state
                        .results
                        .keys()
                        .all(|id| self.nodes.iter().any(|n| &n.id == id)) =>
            {
                state.validate()?;
                if state.limits != *execution.limits() {
                    return Err(AgentError::Output(
                        "Workflow recovery must preserve its run limits.".into(),
                    ));
                }
                execution.absorb_budget(&state.budget);
                execution.restore_sequence(state.sequence);
                state
            }
            Some(_) => {
                return Err(AgentError::Output(
                    "Workflow checkpoint does not match this run.".into(),
                ))
            }
            None => WorkflowState {
                sequence: 0,
                revision: 0,
                error: None,
                plan: self.clone(),
                version: 1,
                run: execution.identity.clone(),
                limits: execution.limits().clone(),
                budget: execution.budget(),
                results: BTreeMap::new(),
            },
        };
        if state.revision > 0 && execution.store().is_none() {
            return Err(AgentError::Output(
                "Persisted workflow recovery requires its store.".into(),
            ));
        }
        state.error = None;
        reconcile(&mut state, &execution).await?;
        persist(&mut state, &execution).await?;
        loop {
            if let Err(error) = execution.check() {
                state.error = Some(error.to_string());
                state.budget = execution.budget();
                persist(&mut state, &execution).await?;
                execution.finish(
                    if matches!(error, AgentError::Stopped(_)) {
                        RunStatus::Cancelled
                    } else {
                        RunStatus::Failed
                    },
                    Some(error.to_string()),
                );
                return Ok(state);
            }
            for node in &self.nodes {
                if state.results.contains_key(&node.id) {
                    continue;
                }
                if let Some(dependency) = node.dependencies.iter().find(|id| {
                    matches!(
                        state.results.get(*id),
                        Some(NodeResult::Failed { .. } | NodeResult::Blocked { .. })
                    )
                }) {
                    state.results.insert(
                        node.id.clone(),
                        NodeResult::Blocked {
                            dependency: dependency.clone(),
                        },
                    );
                }
            }
            let ready: Vec<_> = self
                .nodes
                .iter()
                .filter(|n| {
                    !state.results.contains_key(&n.id)
                        && n.dependencies.iter().all(|id| {
                            matches!(state.results.get(id), Some(NodeResult::Completed { .. }))
                        })
                })
                .collect();
            if ready.is_empty() {
                break;
            }
            // Bound both live node futures and the underlying model/tool IO.
            for batch in ready.chunks(execution.limits().max_concurrency) {
                let mut jobs = Vec::new();
                let mut contexts = Vec::new();
                for node in batch {
                    let mut task =
                        self.agents[&node.agent_id].instantiate(&bindings, node.prompt.clone())?;
                    task.execution = execution.child(
                        node.id.clone(),
                        Some(format!("{}:node:{}", execution.identity.run_id, node.id)),
                    )?;
                    task.agent_definitions = self.agents.clone();
                    for id in &node.dependencies {
                        if let Some(NodeResult::Completed { text, .. }) = state.results.get(id) {
                            task.context
                                .push(AgentContextBlock::new(format!("Dependency {id}"), text));
                        }
                    }
                    state.results.insert(
                        node.id.clone(),
                        NodeResult::Running {
                            run_id: task.execution.identity.run_id.clone(),
                        },
                    );
                    let context = task.execution.clone();
                    let id = node.id.clone();
                    contexts.push((id.clone(), context.clone()));
                    jobs.push(async move {
                        let result = run_agent_task_until_pause(task, |_| Ok(())).await;
                        let result = match result {
                            Ok(AgentTaskRunResult::Completed(outcome)) => NodeResult::Completed {
                                output: outcome.final_value,
                                text: outcome.final_text,
                            },
                            Ok(AgentTaskRunResult::Paused(_)) => NodeResult::WaitingInput {
                                checkpoint: Box::new(
                                    context.checkpoint().expect("paused run has a checkpoint"),
                                ),
                            },
                            Err(error) => NodeResult::Failed {
                                error: error.to_string(),
                                checkpoint: context.checkpoint().map(Box::new),
                            },
                        };
                        (id, result)
                    });
                }
                persist(&mut state, &execution).await?;
                use futures_util::StreamExt;
                let mut jobs = futures_util::stream::FuturesUnordered::from_iter(jobs);
                while let Some((id, result)) = jobs.next().await {
                    let failed = matches!(result, NodeResult::Failed { .. });
                    state.results.insert(id, result);
                    persist(&mut state, &execution).await?;
                    if failed && matches!(self.failure_policy, FailurePolicy::FailFast) {
                        for (_, context) in &contexts {
                            context.cancellation.cancel();
                        }
                        drop(jobs);
                        for (id, context) in &contexts {
                            if matches!(state.results.get(id), Some(NodeResult::Running { .. })) {
                                state.results.insert(
                                    id.clone(),
                                    NodeResult::Failed {
                                        error: "Cancelled after another workflow node failed."
                                            .into(),
                                        checkpoint: context.checkpoint().map(Box::new),
                                    },
                                );
                            }
                        }
                        state.budget = execution.budget();
                        persist(&mut state, &execution).await?;
                        execution.finish(RunStatus::Failed, Some("Workflow node failed.".into()));
                        return Ok(state);
                    }
                }
            }
        }
        state.budget = execution.budget();
        persist(&mut state, &execution).await?;
        execution.finish(
            if state
                .results
                .values()
                .any(|r| matches!(r, NodeResult::WaitingInput { .. }))
            {
                RunStatus::WaitingInput
            } else if state
                .results
                .values()
                .any(|r| matches!(r, NodeResult::Failed { .. } | NodeResult::Blocked { .. }))
            {
                RunStatus::Failed
            } else {
                RunStatus::Completed
            },
            None,
        );
        Ok(state)
    }
}

impl WorkflowState {
    pub fn execution(
        &self,
        store: Option<std::sync::Arc<dyn crate::checkpoint::CheckpointStore>>,
    ) -> RunContext {
        let execution = RunContext::restore(
            self.run.clone(),
            self.limits.clone(),
            self.budget.clone(),
            store,
        );
        execution.restore_sequence(self.sequence);
        execution
    }
    pub async fn resume_node(
        &mut self,
        id: &str,
        bindings: &Bindings,
        action: crate::checkpoint::ResumeAction,
        store: Option<std::sync::Arc<dyn crate::checkpoint::CheckpointStore>>,
    ) -> Result<(), AgentError> {
        let mut checkpoint = match self.results.get(id) {
            Some(
                NodeResult::WaitingInput { checkpoint }
                | NodeResult::Failed {
                    checkpoint: Some(checkpoint),
                    ..
                },
            ) => (**checkpoint).clone(),
            _ => {
                return Err(AgentError::Output(
                    "This workflow node has no recoverable checkpoint.".into(),
                ))
            }
        };
        if self.revision > 0 && store.is_none() {
            return Err(AgentError::Output(
                "Persisted workflow recovery requires its store.".into(),
            ));
        }
        if let Some(store) = &store {
            if let Some(latest) = store.load(&checkpoint.run.run_id).await? {
                if latest.revision >= checkpoint.revision {
                    checkpoint = latest;
                }
            }
        }
        crate::checkpoint::merge_budget(&mut self.budget, &checkpoint.budget);
        checkpoint.budget = self.budget.clone();
        checkpoint.limits = self.limits.clone();
        let workflow_context = self.execution(store.clone());
        if matches!(
            checkpoint.phase,
            crate::checkpoint::CheckpointPhase::Completed
        ) {
            let (_, outcome) = checkpoint.completed_result(store)?;
            self.results.insert(
                id.to_string(),
                NodeResult::Completed {
                    output: outcome.final_value,
                    text: outcome.final_text,
                },
            );
            persist(self, &workflow_context).await?;
            return Ok(());
        }
        let task = match store {
            Some(store) => checkpoint.restore_tree(bindings, action, store).await?,
            None => checkpoint.restore(bindings, action, None)?,
        };
        let context = task.execution.clone();
        let result = match run_agent_task_until_pause(task, |_| Ok(())).await {
            Ok(AgentTaskRunResult::Completed(outcome)) => NodeResult::Completed {
                output: outcome.final_value,
                text: outcome.final_text,
            },
            Ok(AgentTaskRunResult::Paused(_)) => NodeResult::WaitingInput {
                checkpoint: Box::new(context.checkpoint().unwrap()),
            },
            Err(error) => NodeResult::Failed {
                error: error.to_string(),
                checkpoint: context.checkpoint().map(Box::new),
            },
        };
        self.budget = context.budget();
        self.results.insert(id.to_string(), result);
        workflow_context.absorb_budget(&self.budget);
        workflow_context.restore_sequence(context.events_after(u64::MAX).next_sequence);
        persist(self, &workflow_context).await?;
        Ok(())
    }
}

impl WorkflowState {
    pub fn validate(&self) -> Result<(), AgentError> {
        if self.version != 1 {
            return Err(AgentError::Output(
                "Unsupported workflow checkpoint version.".into(),
            ));
        }
        self.limits.validate()?;
        self.plan.validate()?;
        if self
            .results
            .keys()
            .any(|id| !self.plan.nodes.iter().any(|node| &node.id == id))
        {
            return Err(AgentError::Output(
                "Workflow checkpoint contains an unknown node.".into(),
            ));
        }
        Ok(())
    }
}

async fn persist(state: &mut WorkflowState, execution: &RunContext) -> Result<(), AgentError> {
    state.budget = execution.budget();
    state.sequence = execution.events_after(u64::MAX).next_sequence;
    if let Some(store) = execution.store() {
        let previous = (state.revision > 0).then_some(state.revision);
        let mut next = state.clone();
        next.revision = state
            .revision
            .checked_add(1)
            .ok_or_else(|| AgentError::Output("Workflow revision overflow.".into()))?;
        store.save_workflow(&next, previous).await?;
        *state = next;
    }
    Ok(())
}

async fn reconcile(state: &mut WorkflowState, execution: &RunContext) -> Result<(), AgentError> {
    let Some(store) = execution.store() else {
        return Ok(());
    };
    let running: Vec<_> = state
        .results
        .iter()
        .filter_map(|(id, result)| match result {
            NodeResult::Running { run_id } => Some((id.clone(), run_id.clone())),
            NodeResult::WaitingInput { checkpoint }
            | NodeResult::Failed {
                checkpoint: Some(checkpoint),
                ..
            } => Some((id.clone(), checkpoint.run.run_id.clone())),
            _ => None,
        })
        .collect();
    for (id, run_id) in running {
        match store.load(&run_id).await? {
            Some(checkpoint) => {
                if checkpoint.run.parent_run_id.as_deref() != Some(&state.run.run_id) {
                    return Err(AgentError::Output(
                        "Workflow node checkpoint belongs to another parent.".into(),
                    ));
                }
                execution.absorb_budget(&checkpoint.budget);
                execution.seed_checkpoint(checkpoint.clone());
                if let Some(outcome) = checkpoint.outcome.as_ref().filter(|_| {
                    matches!(
                        checkpoint.phase,
                        crate::checkpoint::CheckpointPhase::Completed
                    )
                }) {
                    state.results.insert(
                        id,
                        NodeResult::Completed {
                            text: outcome.final_text.clone(),
                            output: outcome.final_value.clone(),
                        },
                    );
                } else if matches!(
                    checkpoint.phase,
                    crate::checkpoint::CheckpointPhase::WaitingInput
                ) {
                    state.results.insert(
                        id,
                        NodeResult::WaitingInput {
                            checkpoint: Box::new(checkpoint),
                        },
                    );
                } else {
                    state.results.insert(
                        id,
                        NodeResult::Failed {
                            error:
                                "Workflow node was interrupted; choose an explicit recovery action."
                                    .into(),
                            checkpoint: Some(Box::new(checkpoint)),
                        },
                    );
                }
            }
            None if matches!(state.results.get(&id), Some(NodeResult::Running { .. })) => {
                state.results.remove(&id);
            }
            None => {}
        }
    }
    Ok(())
}
