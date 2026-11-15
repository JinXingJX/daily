//! Owned background runs. Dropping a handle cancels its entire execution tree.
use crate::{checkpoint::*, definition::Bindings, execution::*, *};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct RunSupervisor {
    runs: Arc<Mutex<BTreeMap<String, RunContext>>>,
    store: Option<Arc<dyn CheckpointStore>>,
    capacity: Arc<tokio::sync::Semaphore>,
}
impl Default for RunSupervisor {
    fn default() -> Self {
        Self {
            runs: Default::default(),
            store: None,
            capacity: Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }
}
impl RunSupervisor {
    pub fn with_capacity(max_active: usize) -> Result<Self, AgentError> {
        if !(1..=1024).contains(&max_active) {
            return Err(AgentError::Output(
                "Supervisor capacity must be 1-1024.".into(),
            ));
        }
        Ok(Self {
            capacity: Arc::new(tokio::sync::Semaphore::new(max_active)),
            ..Default::default()
        })
    }
    pub fn with_store(store: Arc<dyn CheckpointStore>) -> Self {
        Self {
            store: Some(store),
            ..Default::default()
        }
    }
    pub fn start(&self, task: AgentTask) -> Result<RunHandle, AgentError> {
        self.start_inner(task, false)
    }
    fn start_inner(
        &self,
        mut task: AgentTask,
        replace_terminal: bool,
    ) -> Result<RunHandle, AgentError> {
        if let Some(store) = &self.store {
            if task.execution.store().is_none() {
                task.execution = task.execution.with_store(store.clone())?;
            }
        }
        let context = task.execution.clone();
        let mut runs = self.runs.lock().unwrap();
        if runs.get(&context.identity.run_id).is_some_and(|run| {
            !replace_terminal || matches!(run.status(), RunStatus::Running | RunStatus::Queued)
        }) {
            return Err(AgentError::Output(
                "This supervisor already owns that run id.".into(),
            ));
        }
        runs.insert(context.identity.run_id.clone(), context.clone());
        drop(runs);
        let capacity = self.capacity.clone();
        let worker_context = context.clone();
        let join = tokio::spawn(async move {
            let permit = worker_context.guard(capacity.acquire_owned()).await;
            match permit {
                Ok(Ok(_permit)) => run_agent_task_until_pause(task, |_| Ok(())).await,
                Ok(Err(error)) => {
                    worker_context.finish(RunStatus::Cancelled, Some(error.to_string()));
                    Err(AgentError::Stopped(error.to_string()))
                }
                Err(error) => {
                    worker_context.finish(RunStatus::Cancelled, Some(error.to_string()));
                    Err(error)
                }
            }
        });
        Ok(RunHandle {
            context,
            join: Some(join),
        })
    }
    pub async fn resume(
        &self,
        run_id: &str,
        bindings: &Bindings,
        action: ResumeAction,
    ) -> Result<RunHandle, AgentError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| AgentError::Output("Resume requires checkpoint storage.".into()))?;
        let checkpoint = store
            .load(run_id)
            .await?
            .ok_or_else(|| AgentError::Output("Run checkpoint was not found.".into()))?;
        if self
            .runs
            .lock()
            .unwrap()
            .get(run_id)
            .is_some_and(|r| matches!(r.status(), RunStatus::Running | RunStatus::Queued))
        {
            return Err(AgentError::Output("Run is still executing.".into()));
        }
        let task = checkpoint
            .restore_tree(bindings, action, store.clone())
            .await?;
        self.start_inner(task, true)
    }
    pub fn runs(&self) -> Vec<RunContext> {
        self.runs.lock().unwrap().values().cloned().collect()
    }
    pub fn cancel_all(&self) {
        for run in self.runs() {
            run.cancellation.cancel();
        }
    }
    pub fn forget(&self, run_id: &str) -> Result<(), AgentError> {
        let mut runs = self.runs.lock().unwrap();
        if runs
            .get(run_id)
            .is_some_and(|r| matches!(r.status(), RunStatus::Running | RunStatus::Queued))
        {
            return Err(AgentError::Output(
                "Cancel and wait for the run before forgetting it.".into(),
            ));
        }
        runs.remove(run_id);
        Ok(())
    }
}

pub struct RunHandle {
    pub context: RunContext,
    join: Option<JoinHandle<Result<AgentTaskRunResult, AgentError>>>,
}
impl RunHandle {
    pub fn cancel(&self) {
        self.context.cancellation.cancel();
    }
    pub async fn wait(mut self) -> Result<AgentTaskRunResult, AgentError> {
        let result = self.join.as_mut().unwrap().await;
        self.join.take();
        result.map_err(|error| {
            self.context.finish(
                if error.is_cancelled() {
                    RunStatus::Cancelled
                } else {
                    RunStatus::Failed
                },
                Some(error.to_string()),
            );
            AgentError::Stopped(format!("Run worker ended: {error}"))
        })?
    }
}
impl Drop for RunHandle {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            if !join.is_finished() {
                self.cancel();
                self.context
                    .finish(RunStatus::Cancelled, Some("Run handle dropped.".into()));
                join.abort();
            }
        }
    }
}
