//! Desktop persistence adapter for the standalone SDK's checkpoint contract.
use crate::fs_utils::sandbox::resolve_sandboxed_path;
use daily_agent_core::{
    checkpoint::{CheckpointStore, RunCheckpoint},
    workflow::WorkflowState,
    AgentError, BudgetSnapshot, RunContext,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

const MAX_CHECKPOINT_BYTES: usize = 32 * 1024 * 1024;
const AGENTS: &str = "agent_checkpoints";
const WORKFLOWS: &str = "workflow_checkpoints";

#[derive(Clone)]
pub(crate) struct SqliteCheckpointStore(Arc<Mutex<Connection>>);
impl SqliteCheckpointStore {
    pub(crate) fn open(root: &Path) -> Result<Self, String> {
        let path = resolve_sandboxed_path(root, ".daily/agent-runs.sqlite3")?;
        resolve_sandboxed_path(root, ".daily/agent-runs.sqlite3-wal")?;
        resolve_sandboxed_path(root, ".daily/agent-runs.sqlite3-shm")?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        connection.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS agent_checkpoints (run_id TEXT PRIMARY KEY, revision INTEGER NOT NULL, body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS workflow_checkpoints (run_id TEXT PRIMARY KEY, revision INTEGER NOT NULL, body TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS agent_checkpoint_parent ON agent_checkpoints(json_extract(body, '$.run.parentRunId'));").map_err(|e| e.to_string())?;
        Ok(Self(Arc::new(Mutex::new(connection))))
    }
    // Table names come only from the constants above, never from user input.
    async fn load_raw(&self, table: &'static str, id: &str) -> Result<Option<String>, AgentError> {
        let connection = self.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().map_err(failure)?;
            let raw: Option<(i64, Option<String>)> = connection.query_row(&format!("SELECT length(CAST(body AS BLOB)), CASE WHEN length(CAST(body AS BLOB))<=?2 THEN body END FROM {table} WHERE run_id=?1"), params![id, MAX_CHECKPOINT_BYTES as i64], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(failure)?;
            match raw { Some((_, Some(body))) => Ok(Some(body)), Some(_) => Err(failure("Checkpoint exceeds the 32 MiB limit.")), None => Ok(None) }
        }).await.map_err(failure)?
    }
    async fn save_raw(
        &self,
        table: &'static str,
        id: String,
        revision: u64,
        raw: String,
        expected: Option<u64>,
    ) -> Result<(), AgentError> {
        if revision != expected.unwrap_or(0).saturating_add(1) || revision > i64::MAX as u64 {
            return Err(failure("Invalid revision."));
        }
        if raw.len() > MAX_CHECKPOINT_BYTES {
            return Err(failure("Checkpoint exceeds the 32 MiB limit."));
        }
        let connection = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().map_err(failure)?;
            let changed = match expected {
                None => connection.execute(
                    &format!(
                        "INSERT OR IGNORE INTO {table}(run_id,revision,body) VALUES(?1,?2,?3)"
                    ),
                    params![id, revision as i64, raw],
                ),
                Some(previous) => connection.execute(
                    &format!(
                        "UPDATE {table} SET revision=?2,body=?3 WHERE run_id=?1 AND revision=?4"
                    ),
                    params![id, revision as i64, raw, previous as i64],
                ),
            }
            .map_err(failure)?;
            if changed != 1 {
                return Err(failure("Stale checkpoint writer."));
            }
            Ok(())
        })
        .await
        .map_err(failure)?
    }
    /// All snapshots hold aggregate counters. Max-merge the tree, including sibling hooks,
    /// so a resumed primary run cannot reset usage already spent by a later hook.
    pub(crate) async fn tree_budget(&self, run_id: &str) -> Result<BudgetSnapshot, AgentError> {
        let connection = self.0.clone();
        let id = run_id.to_string();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().map_err(failure)?;
            let mut root = id;
            for _ in 0..=32 {
                let parent: Option<Option<String>> = connection.query_row("SELECT json_extract(body, '$.run.parentRunId') FROM agent_checkpoints WHERE run_id=?1", [&root], |row| row.get(0)).optional().map_err(failure)?;
                match parent.flatten() { Some(parent) if parent != root => root = parent, _ => break }
            }
            let mut statement = connection.prepare("WITH RECURSIVE tree(id) AS (SELECT ?1 UNION SELECT run_id FROM agent_checkpoints JOIN tree ON json_extract(body, '$.run.parentRunId')=tree.id) SELECT json_extract(body, '$.budget') FROM agent_checkpoints WHERE run_id IN (SELECT id FROM tree)").map_err(failure)?;
            let budgets = statement.query_map([root], |row| row.get::<_, String>(0)).map_err(failure)?;
            let context = RunContext::new("budget-recovery", Default::default());
            for budget in budgets { context.absorb_budget(&serde_json::from_str::<BudgetSnapshot>(&budget.map_err(failure)?).map_err(failure)?); }
            Ok(context.budget())
        }).await.map_err(failure)?
    }
}
fn failure(error: impl std::fmt::Display) -> AgentError {
    AgentError::Output(format!("Agent checkpoint storage failed: {error}"))
}

#[async_trait::async_trait]
impl CheckpointStore for SqliteCheckpointStore {
    async fn load(&self, run_id: &str) -> Result<Option<RunCheckpoint>, AgentError> {
        self.load_raw(AGENTS, run_id)
            .await?
            .map(|raw| {
                let checkpoint: RunCheckpoint = serde_json::from_str(&raw).map_err(failure)?;
                checkpoint.validate()?;
                Ok(checkpoint)
            })
            .transpose()
    }
    async fn load_tree_budget(&self, run_id: &str) -> Result<BudgetSnapshot, AgentError> {
        self.tree_budget(run_id).await
    }
    async fn save(
        &self,
        checkpoint: &RunCheckpoint,
        expected: Option<u64>,
    ) -> Result<(), AgentError> {
        checkpoint.validate()?;
        self.save_raw(
            AGENTS,
            checkpoint.run.run_id.clone(),
            checkpoint.revision,
            serde_json::to_string(checkpoint).map_err(failure)?,
            expected,
        )
        .await
    }
    async fn load_workflow(&self, id: &str) -> Result<Option<WorkflowState>, AgentError> {
        self.load_raw(WORKFLOWS, id)
            .await?
            .map(|raw| {
                let state: WorkflowState = serde_json::from_str(&raw).map_err(failure)?;
                state.validate()?;
                Ok(state)
            })
            .transpose()
    }
    async fn save_workflow(
        &self,
        state: &WorkflowState,
        expected: Option<u64>,
    ) -> Result<(), AgentError> {
        state.validate()?;
        self.save_raw(
            WORKFLOWS,
            state.run.run_id.clone(),
            state.revision,
            serde_json::to_string(state).map_err(failure)?,
            expected,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daily_agent_core::{model::*, *};
    struct Offline;
    #[async_trait::async_trait]
    impl Model for Offline {
        fn identity(&self) -> ModelIdentity {
            ModelIdentity {
                provider: "offline".into(),
                model: "fixture".into(),
            }
        }
        async fn complete(
            &self,
            _: &ModelCompletionRequest,
            _: Option<&mut (dyn FnMut(ModelStreamChunk) -> Result<(), String> + Send)>,
        ) -> Result<ModelCompletionResponse, ModelError> {
            Ok(ModelCompletionResponse {
                raw_output: "done".into(),
                ..Default::default()
            })
        }
    }
    #[tokio::test]
    async fn reopen_preserves_checkpoints_and_stale_connections_cannot_overwrite_them() {
        let root = std::env::temp_dir().join(format!(
            "daily-sdk-checkpoints-{}",
            crate::util::unique_stamp()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store = Arc::new(SqliteCheckpointStore::open(&root).unwrap());
        let mut task = AgentTask::new("persist", Arc::new(Offline), "work");
        task.execution = RunContext::new_with_store("persist", Default::default(), store.clone());
        let context = task.execution.clone();
        run_agent_task(task, |_| Ok(())).await.unwrap();
        let checkpoint = store.load(&context.identity.run_id).await.unwrap().unwrap();
        assert!(matches!(
            checkpoint.phase,
            daily_agent_core::checkpoint::CheckpointPhase::Completed
        ));
        let reopened = SqliteCheckpointStore::open(&root).unwrap();
        assert_eq!(
            reopened
                .load(&checkpoint.run.run_id)
                .await
                .unwrap()
                .unwrap()
                .outcome
                .as_ref()
                .unwrap()
                .final_text,
            "done"
        );
        let mut next = checkpoint.clone();
        next.revision += 1;
        reopened
            .save(&next, Some(checkpoint.revision))
            .await
            .unwrap();
        assert!(store.save(&next, Some(checkpoint.revision)).await.is_err());
        assert_eq!(
            store
                .load(&checkpoint.run.run_id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            next.revision
        );
    }
    #[tokio::test]
    async fn workflow_reopen_and_sibling_budgets_survive_restart() {
        use daily_agent_core::{workflow::*, *};
        let root =
            std::env::temp_dir().join(format!("daily-sdk-graph-{}", crate::util::unique_stamp()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Arc::new(SqliteCheckpointStore::open(&root).unwrap());
        let mut bindings = Bindings::default();
        bindings.models.insert("default".into(), Arc::new(Offline));
        let definition = AgentDefinition::new(AgentToolSpec {
            id: "worker".into(),
            name: "Worker".into(),
            description: "worker".into(),
            instructions: "work".into(),
            tools: vec![],
            max_iterations: Some(2),
        });
        let workflow = Workflow::serial(
            std::collections::BTreeMap::from([("worker".into(), definition)]),
            (0..2)
                .map(|i| WorkflowNode {
                    id: i.to_string(),
                    agent_id: "worker".into(),
                    prompt: "work".into(),
                    dependencies: vec![],
                })
                .collect(),
        );
        let execution = RunContext::new_with_store("workflow", Default::default(), store.clone());
        let id = execution.identity.run_id.clone();
        let state = workflow.run(&bindings, execution, None).await.unwrap();
        assert_eq!(state.budget.model_requests, 2);
        let reopened = SqliteCheckpointStore::open(&root).unwrap();
        let saved = reopened.load_workflow(&id).await.unwrap().unwrap();
        assert_eq!(saved.results.len(), 2);
        assert_eq!(saved.revision, state.revision);
        let mut next = saved.clone();
        next.revision += 1;
        reopened
            .save_workflow(&next, Some(saved.revision))
            .await
            .unwrap();
        assert!(store
            .save_workflow(&next, Some(saved.revision))
            .await
            .is_err());
        // First child only recorded one request, but its sibling spent another.
        let child = format!("{id}:node:0");
        assert_eq!(
            reopened
                .load(&child)
                .await
                .unwrap()
                .unwrap()
                .budget
                .model_requests,
            1
        );
        assert_eq!(
            reopened
                .load_tree_budget(&child)
                .await
                .unwrap()
                .model_requests,
            2
        );
    }
}
