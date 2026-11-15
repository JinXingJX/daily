//! Serializable agent definitions and executable bindings supplied by a host.
use crate::{model::Model, AgentError, AgentTask, AgentTool, AgentToolSpec};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    pub spec: AgentToolSpec,
    #[serde(default)]
    pub model_key: Option<String>,
    #[serde(default)]
    pub delegates: Vec<String>,
    #[serde(default)]
    pub allow_create: bool,
    #[serde(default)]
    pub allow_handoff: bool,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}
impl AgentDefinition {
    pub fn new(spec: AgentToolSpec) -> Self {
        Self {
            spec,
            model_key: None,
            delegates: Vec::new(),
            allow_create: false,
            allow_handoff: false,
            temperature: None,
            max_output_tokens: None,
        }
    }
    pub fn with_model(mut self, key: impl Into<String>) -> Self {
        self.model_key = Some(key.into());
        self
    }
    pub fn delegate(mut self, id: impl Into<String>) -> Self {
        self.delegates.push(id.into());
        self
    }
    pub fn instantiate(
        &self,
        bindings: &Bindings,
        prompt: impl Into<String>,
    ) -> Result<AgentTask, AgentError> {
        let key = self.model_key.as_deref().unwrap_or("default");
        let mut task = AgentTask::new(self.spec.id.clone(), bindings.model(key)?, prompt);
        task.system_prompt = self.spec.instructions.clone();
        task.tools = bindings.tools(&self.spec.tools)?;
        task.max_iterations = self.spec.max_iterations.unwrap_or(24);
        task.bindings = bindings.clone();
        task.agent_definitions = bindings.agents.clone();
        task.sub_agents.allow_create = self.allow_create;
        task.allow_handoff = self.allow_handoff;
        task.temperature = self.temperature.unwrap_or(task.temperature);
        task.max_output_tokens = self.max_output_tokens.unwrap_or(task.max_output_tokens);
        for id in &self.delegates {
            task.sub_agents.agents.push(
                bindings
                    .agents
                    .get(id)
                    .ok_or_else(|| AgentError::Output(format!("Delegate {id} is unavailable.")))?
                    .spec
                    .clone(),
            );
        }
        Ok(task)
    }
}

#[derive(Clone, Default)]
pub struct Bindings {
    pub models: BTreeMap<String, Arc<dyn Model>>,
    pub tools: BTreeMap<String, AgentTool>,
    pub agents: BTreeMap<String, AgentDefinition>,
}
impl Bindings {
    pub fn model(&self, key: &str) -> Result<Arc<dyn Model>, AgentError> {
        self.models
            .get(key)
            .cloned()
            .ok_or_else(|| AgentError::Output(format!("Model binding {key} is unavailable.")))
    }
    pub fn tools(&self, ids: &[String]) -> Result<Vec<AgentTool>, AgentError> {
        ids.iter()
            .map(|id| {
                self.tools
                    .get(id)
                    .cloned()
                    .ok_or_else(|| AgentError::Output(format!("Tool binding {id} is unavailable.")))
            })
            .collect()
    }
}
