//! Run identity, cooperative cancellation, shared limits, and bounded event replay.
use crate::{
    checkpoint::{CheckpointStore, RunCheckpoint},
    model::*,
    AgentError, AgentTaskEvent,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, watch, Semaphore};

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Clone)]
pub struct Cancellation {
    signal: watch::Sender<bool>,
    ancestors: Vec<watch::Receiver<bool>>,
}
impl Default for Cancellation {
    fn default() -> Self {
        Self {
            signal: watch::channel(false).0,
            ancestors: Vec::new(),
        }
    }
}
impl Cancellation {
    pub fn cancel(&self) {
        self.signal.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.signal.borrow() || self.ancestors.iter().any(|r| *r.borrow())
    }
    pub fn child(&self) -> Self {
        let mut ancestors = self.ancestors.clone();
        ancestors.push(self.signal.subscribe());
        Self {
            signal: watch::channel(false).0,
            ancestors,
        }
    }
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let mut receivers = self.ancestors.clone();
        receivers.push(self.signal.subscribe());
        let waits = receivers.into_iter().map(|mut receiver| async move {
            let _ = receiver.wait_for(|value| *value).await;
        });
        use futures_util::StreamExt;
        futures_util::stream::FuturesUnordered::from_iter(waits)
            .next()
            .await;
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunLimits {
    pub max_model_requests: u64,
    pub max_tool_calls: u64,
    pub max_concurrency: usize,
    pub max_depth: usize,
    /// Stops further requests once reported aggregate usage reaches this value.
    pub max_reported_tokens: Option<u64>,
    /// Absolute deadline, preserved across process restarts.
    pub deadline_unix_ms: Option<u64>,
}
impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_model_requests: 256,
            max_tool_calls: 1024,
            max_concurrency: 4,
            max_depth: 8,
            max_reported_tokens: None,
            deadline_unix_ms: None,
        }
    }
}
impl RunLimits {
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.deadline_unix_ms =
            Some(now_ms().saturating_add(timeout.as_millis().min(u64::MAX as u128) as u64));
        self
    }
    pub fn validate(&self) -> Result<(), AgentError> {
        if self.max_model_requests == 0
            || self.max_tool_calls == 0
            || self.max_concurrency == 0
            || self.max_concurrency > 1024
            || self.max_depth > 32
        {
            return Err(AgentError::Output(
                "Run limits require non-zero budgets, concurrency 1-1024 and depth at most 32."
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetSnapshot {
    pub model_requests: u64,
    pub tool_calls: u64,
    pub usage: ModelUsage,
    pub responses_without_usage: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    WaitingInput,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunIdentity {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub agent_id: String,
    pub depth: usize,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    Agent {
        event: Box<AgentTaskEvent>,
    },
    Status {
        status: RunStatus,
        error: Option<String>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEventEnvelope {
    pub sequence: u64,
    pub run: RunIdentity,
    pub event: RunEvent,
}

#[derive(Clone)]
pub struct EventPage {
    pub events: Vec<RunEventEnvelope>,
    pub next_sequence: u64,
    pub truncated: bool,
}

struct Shared {
    limits: RunLimits,
    budget: Mutex<BudgetSnapshot>,
    permits: Semaphore,
    sequence: AtomicU64,
    events: Mutex<VecDeque<RunEventEnvelope>>,
    sender: broadcast::Sender<RunEventEnvelope>,
    checkpoints: Mutex<BTreeMap<String, RunCheckpoint>>,
    checkpoint_gates: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
    statuses: Mutex<BTreeMap<String, RunStatus>>,
    store: Mutex<Option<Arc<dyn CheckpointStore>>>,
}

#[derive(Clone)]
pub struct RunContext {
    pub identity: RunIdentity,
    pub cancellation: Cancellation,
    shared: Arc<Shared>,
}

impl RunContext {
    pub fn new(agent_id: impl Into<String>, limits: RunLimits) -> Self {
        Self::restore(
            RunIdentity {
                run_id: fresh_id(),
                parent_run_id: None,
                agent_id: agent_id.into(),
                depth: 0,
            },
            limits,
            BudgetSnapshot::default(),
            None,
        )
    }
    pub fn new_with_store(
        agent_id: impl Into<String>,
        limits: RunLimits,
        store: Arc<dyn CheckpointStore>,
    ) -> Self {
        let context = Self::new(agent_id, limits);
        Self::restore(
            context.identity,
            context.shared.limits.clone(),
            BudgetSnapshot::default(),
            Some(store),
        )
    }
    pub fn store(&self) -> Option<Arc<dyn CheckpointStore>> {
        self.shared.store.lock().unwrap().clone()
    }
    pub(crate) fn restore(
        identity: RunIdentity,
        limits: RunLimits,
        budget: BudgetSnapshot,
        store: Option<Arc<dyn CheckpointStore>>,
    ) -> Self {
        Self {
            identity,
            cancellation: Cancellation::default(),
            shared: Arc::new(Shared {
                permits: Semaphore::new(limits.max_concurrency.clamp(1, 1024)),
                limits,
                budget: Mutex::new(budget),
                sequence: AtomicU64::new(0),
                events: Mutex::new(VecDeque::new()),
                sender: broadcast::channel(256).0,
                checkpoints: Mutex::new(BTreeMap::new()),
                checkpoint_gates: Mutex::new(BTreeMap::new()),
                statuses: Mutex::new(BTreeMap::new()),
                store: Mutex::new(store),
            }),
        }
    }
    pub fn with_store(self, store: Arc<dyn CheckpointStore>) -> Result<Self, AgentError> {
        if !self.shared.statuses.lock().unwrap().is_empty() {
            return Err(AgentError::Output(
                "Attach checkpoint storage before starting a run.".into(),
            ));
        }
        *self.shared.store.lock().unwrap() = Some(store);
        Ok(self)
    }
    pub fn child(
        &self,
        agent_id: impl Into<String>,
        run_id: Option<String>,
    ) -> Result<Self, AgentError> {
        if self.identity.depth >= self.shared.limits.max_depth {
            return Err(AgentError::Budget(
                "Maximum agent nesting depth reached.".into(),
            ));
        }
        Ok(Self {
            identity: RunIdentity {
                run_id: run_id.unwrap_or_else(fresh_id),
                parent_run_id: Some(self.identity.run_id.clone()),
                agent_id: agent_id.into(),
                depth: self.identity.depth + 1,
            },
            cancellation: self.cancellation.child(),
            shared: self.shared.clone(),
        })
    }
    pub fn limits(&self) -> &RunLimits {
        &self.shared.limits
    }
    pub fn absorb_budget(&self, source: &BudgetSnapshot) {
        let mut target = self.shared.budget.lock().unwrap();
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
    pub fn budget(&self) -> BudgetSnapshot {
        self.shared.budget.lock().unwrap().clone()
    }
    pub fn subscribe(&self) -> broadcast::Receiver<RunEventEnvelope> {
        self.shared.sender.subscribe()
    }
    pub fn events_after(&self, sequence: u64) -> EventPage {
        let events = self.shared.events.lock().unwrap();
        EventPage {
            truncated: events
                .front()
                .is_some_and(|e| sequence.saturating_add(1) < e.sequence),
            events: events
                .iter()
                .filter(|e| e.sequence > sequence)
                .cloned()
                .collect(),
            next_sequence: self.shared.sequence.load(Ordering::SeqCst),
        }
    }
    pub fn status(&self) -> RunStatus {
        self.shared
            .statuses
            .lock()
            .unwrap()
            .get(&self.identity.run_id)
            .cloned()
            .unwrap_or(RunStatus::Queued)
    }
    pub(crate) fn begin(&self) -> Result<(), AgentError> {
        self.limits().validate()?;
        self.check()?;
        let mut statuses = self.shared.statuses.lock().unwrap();
        if statuses.contains_key(&self.identity.run_id) {
            return Err(AgentError::Output(
                "This run must be restored from its checkpoint before executing again.".into(),
            ));
        }
        statuses.insert(self.identity.run_id.clone(), RunStatus::Running);
        drop(statuses);
        self.publish(RunEvent::Status {
            status: RunStatus::Running,
            error: None,
        });
        Ok(())
    }
    pub(crate) fn finish(&self, status: RunStatus, error: Option<String>) {
        self.shared
            .statuses
            .lock()
            .unwrap()
            .insert(self.identity.run_id.clone(), status.clone());
        self.publish(RunEvent::Status { status, error });
    }
    pub(crate) fn publish(&self, event: RunEvent) {
        // Allocate sequences while holding the replay lock so parallel publishers retain order.
        let mut history = self.shared.events.lock().unwrap();
        let envelope = RunEventEnvelope {
            sequence: self.shared.sequence.fetch_add(1, Ordering::SeqCst) + 1,
            run: self.identity.clone(),
            event,
        };
        history.push_back(envelope.clone());
        if history.len() > 1024 {
            history.pop_front();
        }
        let _ = self.shared.sender.send(envelope);
    }
    pub fn checkpoint(&self) -> Option<RunCheckpoint> {
        self.shared
            .checkpoints
            .lock()
            .unwrap()
            .get(&self.identity.run_id)
            .cloned()
    }
    pub(crate) async fn save(&self, mut checkpoint: RunCheckpoint) -> Result<(), AgentError> {
        let gate = self
            .shared
            .checkpoint_gates
            .lock()
            .unwrap()
            .entry(self.identity.run_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = gate.lock().await;
        let previous = self
            .shared
            .checkpoints
            .lock()
            .unwrap()
            .get(&self.identity.run_id)
            .map(|c| c.revision);
        checkpoint.revision = previous
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| AgentError::Output("Checkpoint revision overflow.".into()))?;
        checkpoint.sequence = self.shared.sequence.load(Ordering::SeqCst);
        if let Some(store) = self.store() {
            store.save(&checkpoint, previous).await?;
        }
        self.shared
            .checkpoints
            .lock()
            .unwrap()
            .insert(self.identity.run_id.clone(), checkpoint);
        Ok(())
    }
    pub(crate) fn restore_sequence(&self, sequence: u64) {
        self.shared.sequence.fetch_max(sequence, Ordering::SeqCst);
    }
    pub(crate) fn seed_checkpoint(&self, checkpoint: RunCheckpoint) {
        self.restore_sequence(checkpoint.sequence);
        self.shared
            .checkpoints
            .lock()
            .unwrap()
            .insert(checkpoint.run.run_id.clone(), checkpoint);
    }
    pub fn check(&self) -> Result<(), AgentError> {
        if self.cancellation.is_cancelled() {
            return Err(AgentError::Stopped("Run cancelled.".into()));
        }
        if self
            .limits()
            .deadline_unix_ms
            .is_some_and(|d| now_ms() >= d)
        {
            return Err(AgentError::Budget("Run deadline exceeded.".into()));
        }
        Ok(())
    }
    pub async fn guard<F: Future>(&self, future: F) -> Result<F::Output, AgentError> {
        self.check()?;
        let remaining = self
            .limits()
            .deadline_unix_ms
            .map(|d| Duration::from_millis(d.saturating_sub(now_ms())));
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(AgentError::Stopped("Run cancelled.".into())),
            _ = async { match remaining { Some(d) => tokio::time::sleep(d).await, None => std::future::pending().await } } => Err(AgentError::Budget("Run deadline exceeded.".into())),
            result = future => Ok(result),
        }
    }
    pub(crate) async fn complete(
        &self,
        model: &dyn Model,
        request: &ModelCompletionRequest,
        callback: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
    ) -> Result<ModelCompletionResponse, AgentError> {
        let _permit = self
            .guard(self.shared.permits.acquire())
            .await?
            .map_err(|_| AgentError::Stopped("Run executor closed.".into()))?;
        {
            let mut budget = self.shared.budget.lock().unwrap();
            if budget.model_requests >= self.limits().max_model_requests
                || self.limits().max_reported_tokens.is_some_and(|max| {
                    budget
                        .usage
                        .input_tokens
                        .saturating_add(budget.usage.output_tokens)
                        >= max
                })
            {
                return Err(AgentError::Budget(
                    "Shared model request or token budget exhausted.".into(),
                ));
            }
            budget.model_requests += 1;
        }
        self.flush_budget().await?;
        let result = self
            .guard(model.complete(request, callback))
            .await?
            .map_err(|e| {
                if e.kind == ModelErrorKind::Aborted {
                    AgentError::Stopped(e.message)
                } else {
                    AgentError::Model(e)
                }
            })?;
        {
            let mut budget = self.shared.budget.lock().unwrap();
            match &result.usage {
                Some(usage) => budget.usage.add(usage),
                None => budget.responses_without_usage += 1,
            }
        }
        self.flush_budget().await?;
        Ok(result)
    }
    pub(crate) async fn persist_latest(&self) -> Result<(), AgentError> {
        self.flush_budget().await
    }
    async fn flush_budget(&self) -> Result<(), AgentError> {
        if let Some(mut checkpoint) = self.checkpoint() {
            checkpoint.budget = self.budget();
            self.save(checkpoint).await?;
        }
        Ok(())
    }
    pub(crate) async fn reserve_tool(&self) -> Result<(), AgentError> {
        self.check()?;
        {
            let mut budget = self.shared.budget.lock().unwrap();
            if budget.tool_calls >= self.limits().max_tool_calls {
                return Err(AgentError::Budget(
                    "Shared tool call budget exhausted.".into(),
                ));
            }
            budget.tool_calls += 1;
        }
        self.flush_budget().await
    }
    pub(crate) async fn tool<F: Future>(&self, future: F) -> Result<F::Output, AgentError> {
        let _permit = self
            .guard(self.shared.permits.acquire())
            .await?
            .map_err(|_| AgentError::Stopped("Run executor closed.".into()))?;
        self.reserve_tool().await?;
        self.guard(future).await
    }
}

fn fresh_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "run-{}-{}-{}",
        now_ms(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
