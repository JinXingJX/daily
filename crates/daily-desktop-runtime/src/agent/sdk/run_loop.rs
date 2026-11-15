//! Compatibility adapter: desktop configuration is resolved before entering the SDK.
use super::*;
use crate::model::{
    adapter_for, complete_with_retry, ModelCompletionRequest, ModelCompletionResponse, ModelError,
};

pub(crate) struct DesktopModel<'a> {
    pub http: &'a Client,
    pub config: &'a ModelConfig,
    pub adapter: &'a dyn ModelAdapter,
    pub session_id: Option<u64>,
    pub turn_id: Option<u64>,
}

#[async_trait::async_trait]
impl daily_agent_core::model::Model for DesktopModel<'_> {
    fn identity(&self) -> daily_agent_core::model::ModelIdentity {
        let mut config = self.config.provider_config();
        config.provider_kind = self.adapter.kind();
        daily_agent_providers::model_identity(&config)
    }
    fn supports_images(&self) -> bool {
        self.adapter.supports_images()
    }
    fn supports_streaming(&self) -> bool {
        self.adapter.supports_streaming()
    }
    async fn complete(
        &self,
        request: &daily_agent_core::model::ModelCompletionRequest,
        on_chunk: Option<
            &mut (dyn FnMut(daily_agent_core::model::ModelStreamChunk) -> Result<(), String>
                      + Send),
        >,
    ) -> Result<ModelCompletionResponse, ModelError> {
        // Summaries historically ran outside the chat turn's provider correlation context.
        let (session_id, turn_id) =
            if request.purpose == daily_agent_core::model::ModelRequestPurpose::ContextCompaction {
                (None, None)
            } else {
                (self.session_id, self.turn_id)
            };
        complete_with_retry(
            self.adapter,
            self.http,
            self.config,
            &ModelCompletionRequest {
                messages: request.messages.clone(),
                temperature: request.temperature,
                max_output_tokens: request.max_output_tokens,
                tools: request.tools.clone(),
                tool_choice: request.tool_choice,
                session_id,
                turn_id,
                request_name: request.request_name.clone(),
            },
            on_chunk,
        )
        .await
    }
}

struct OwnedDesktopModel {
    http: Client,
    config: ModelConfig,
    adapter: Arc<dyn ModelAdapter>,
    session_id: Option<u64>,
    turn_id: Option<u64>,
}

#[async_trait::async_trait]
impl daily_agent_core::model::Model for OwnedDesktopModel {
    fn identity(&self) -> daily_agent_core::model::ModelIdentity {
        self.borrowed().identity()
    }
    fn supports_images(&self) -> bool {
        self.adapter.supports_images()
    }
    fn supports_streaming(&self) -> bool {
        self.adapter.supports_streaming()
    }
    async fn complete(
        &self,
        request: &daily_agent_core::model::ModelCompletionRequest,
        on_chunk: Option<
            &mut (dyn FnMut(daily_agent_core::model::ModelStreamChunk) -> Result<(), String>
                      + Send),
        >,
    ) -> Result<ModelCompletionResponse, ModelError> {
        self.borrowed().complete(request, on_chunk).await
    }
}

impl OwnedDesktopModel {
    fn borrowed(&self) -> DesktopModel<'_> {
        DesktopModel {
            http: &self.http,
            config: &self.config,
            adapter: self.adapter.as_ref(),
            session_id: self.session_id,
            turn_id: self.turn_id,
        }
    }
}

impl AgentTask {
    async fn into_core(
        self,
        http: &Client,
    ) -> Result<(daily_agent_core::AgentTask, Option<AgentTaskOutcome>), AgentError> {
        let adapter = self
            .model_adapter
            .unwrap_or_else(|| Arc::from(adapter_for(self.model.provider_kind)));
        let mut task = daily_agent_core::AgentTask {
            execution: self.execution.unwrap_or_else(|| {
                daily_agent_core::RunContext::new(self.name.clone(), Default::default())
            }),
            bindings: daily_agent_core::Bindings {
                models: self.model_bindings,
                ..Default::default()
            },
            agent_definitions: self.agent_definitions,
            pending_round: None,
            model: Arc::new(OwnedDesktopModel {
                http: http.clone(),
                config: self.model,
                adapter,
                session_id: self.session_id,
                turn_id: self.turn_id,
            }),
            name: self.name,
            system_prompt: self.system_prompt,
            user_prompt: self.user_prompt,
            final_instructions: self.final_instructions,
            context: self.context,
            conversation: self.conversation,
            prior_tool_observations: self.prior_tool_observations,
            tools: self.tools,
            max_iterations: self.max_iterations,
            start_ply: self.start_ply,
            observation_budget: self.observation_budget,
            max_output_tokens: self.max_output_tokens,
            temperature: self.temperature,
            inline_images: self.inline_images,
            stream: self.stream,
            require_tool_before_final: self.require_tool_before_final,
            continue_on_tool_error: self.continue_on_tool_error,
            sub_agents: self.sub_agents,
            allow_handoff: self.allow_handoff,
            resume_sub_agent: self.resume_sub_agent,
        };
        task.bindings = task.bindings();
        let checkpoint = match self.checkpoint {
            Some(checkpoint) => Some(checkpoint),
            None => match self.checkpoint_ref {
                Some(id) => Some(
                    task.execution
                        .store()
                        .ok_or_else(|| {
                            AgentError::Output("Checkpoint reference requires storage.".into())
                        })?
                        .load(&id)
                        .await?
                        .ok_or_else(|| {
                            AgentError::Output("Run checkpoint was not found.".into())
                        })?,
                ),
                None => match task.execution.store() {
                    Some(store) => store.load(&task.execution.identity.run_id).await?,
                    None => None,
                },
            },
        };
        if let Some(mut checkpoint) = checkpoint {
            let mut action = self.resume_action;
            if let Some(store) = task.execution.store() {
                if let Some(latest) = store.load(&checkpoint.run.run_id).await? {
                    if latest.revision >= checkpoint.revision {
                        checkpoint = latest;
                    }
                }
                let budget = store.load_tree_budget(&checkpoint.run.run_id).await?;
                let context =
                    daily_agent_core::RunContext::new("budget-recovery", Default::default());
                context.absorb_budget(&checkpoint.budget);
                context.absorb_budget(&budget);
                checkpoint.budget = context.budget();
                if matches!(action, daily_agent_core::checkpoint::ResumeAction::Continue) {
                    action = match checkpoint.phase {
                        daily_agent_core::checkpoint::CheckpointPhase::ModelInFlight
                        | daily_agent_core::checkpoint::CheckpointPhase::Delegating => {
                            daily_agent_core::checkpoint::ResumeAction::RetryModel
                        }
                        daily_agent_core::checkpoint::CheckpointPhase::ToolInFlight {
                            replay_safe: true,
                            ..
                        } => daily_agent_core::checkpoint::ResumeAction::RetrySafeTool,
                        _ => action,
                    };
                }
            }
            if matches!(
                checkpoint.phase,
                daily_agent_core::checkpoint::CheckpointPhase::Completed
            ) {
                let (execution, outcome) = checkpoint.completed_result(task.execution.store())?;
                task.execution = execution;
                return Ok((task, Some(outcome)));
            }
            let restored = match task.execution.store() {
                Some(store) => {
                    checkpoint
                        .restore_tree(&task.bindings, action, store)
                        .await?
                }
                None => checkpoint.restore(&task.bindings, action, None)?,
            };
            Ok((restored, None))
        } else {
            Ok((task, None))
        }
    }
    pub async fn prepare(self, http: &Client) -> Result<PreparedTask, AgentError> {
        let provider = self
            .model_adapter
            .as_ref()
            .map(|m| m.kind())
            .unwrap_or(self.model.provider_kind);
        let (task, outcome) = self.into_core(http).await?;
        Ok(PreparedTask {
            task,
            outcome,
            provider,
        })
    }
}

pub struct PreparedTask {
    task: daily_agent_core::AgentTask,
    outcome: Option<AgentTaskOutcome>,
    provider: ModelProviderKind,
}
impl PreparedTask {
    pub fn execution(&self) -> daily_agent_core::RunContext {
        self.task.execution.clone()
    }
    pub async fn run_until_pause<F>(self, mut emit: F) -> Result<AgentTaskRunResult, AgentError>
    where
        F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
    {
        if let Some(outcome) = self.outcome {
            return Ok(AgentTaskRunResult::Completed(outcome));
        }
        let provider = self.provider;
        daily_agent_core::run_agent_task_until_pause(self.task, |event| {
            emit(AgentTaskEvent::from_core(event, provider))
        })
        .await
    }
}

pub async fn run_agent_task<F>(
    http: &Client,
    task: AgentTask,
    emit: F,
) -> Result<AgentTaskOutcome, AgentError>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
{
    match task.prepare(http).await?.run_until_pause(emit).await? {
        AgentTaskRunResult::Completed(outcome) => Ok(outcome),
        AgentTaskRunResult::Paused(pause) => Err(AgentError::Output(format!(
            "Agent task paused while waiting for async tool {} ({}).",
            pause.tool_name, pause.tool_id
        ))),
    }
}
pub async fn run_agent_task_until_pause<F>(
    http: &Client,
    task: AgentTask,
    emit: F,
) -> Result<AgentTaskRunResult, AgentError>
where
    F: FnMut(AgentTaskEvent) -> Result<(), String> + Send,
{
    task.prepare(http).await?.run_until_pause(emit).await
}

#[cfg(test)]
pub(super) use daily_agent_core::transcript::transcript_messages;
#[cfg(test)]
pub(super) fn build_task_messages(
    task: &AgentTask,
    observations: &[AgentToolObservation],
    force_final: bool,
) -> Vec<ModelMessage> {
    let mut input = daily_agent_core::AgentTask::new(
        task.name.clone(),
        Arc::new(OwnedDesktopModel {
            http: Client::new(),
            config: task.model.clone(),
            adapter: Arc::from(adapter_for(task.model.provider_kind)),
            session_id: task.session_id,
            turn_id: task.turn_id,
        }),
        task.user_prompt.clone(),
    );
    input.system_prompt = task.system_prompt.clone();
    input.final_instructions = task.final_instructions.clone();
    input.context = task.context.clone();
    input.conversation = task.conversation.clone();
    input.inline_images = task.inline_images.clone();
    input.require_tool_before_final = task.require_tool_before_final;
    daily_agent_core::transcript::build_task_messages(&input, observations, force_final)
}
