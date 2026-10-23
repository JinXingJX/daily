use crate::agent::AgentStore;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

pub(crate) type EventCallback = dyn Fn(&str, Value) -> Result<(), String> + Send + Sync;

#[derive(Clone)]
pub struct RuntimeContext {
    pub(crate) store: Arc<AgentStore>,
    pub(crate) emit_event: Arc<EventCallback>,
}

impl RuntimeContext {
    pub(crate) fn new(store: Arc<AgentStore>, emit_event: Arc<EventCallback>) -> Self {
        Self { store, emit_event }
    }

    pub(crate) fn emit<T: Serialize>(&self, event: &str, payload: T) -> Result<(), String> {
        let payload = serde_json::to_value(payload)
            .map_err(|error| format!("Failed to serialize {event}: {error}"))?;
        (self.emit_event)(event, payload)
    }

    pub(crate) fn state(&self) -> Arc<AgentStore> {
        self.store.clone()
    }
}
