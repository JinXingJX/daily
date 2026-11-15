//! Workspace-owned durable tasks. UI connections are observers, never owners.
use crate::fs_utils::sandbox::resolve_sandboxed_path;
use crate::util::{unique_stamp, unix_timestamp};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

const LEASE_SECONDS: u64 = 45;
const MAX_VALUE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ACTIVE: usize = 4;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Task {
    #[serde(default)]
    pub title: String,
    pub id: String,
    pub plugin_id: String,
    pub kind: String,
    pub scope: String,
    pub request_key: String,
    pub exclusive_key: Option<String>,
    pub status: String,
    pub attempt: u64,
    pub cursor: u64,
    pub progress: Value,
    #[serde(default)]
    pub result: Value,
    pub error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    input: Value,
    #[serde(default)]
    checkpoint: Value,
    #[serde(default)]
    resume: Value,
    #[serde(default)]
    lease_token: Option<String>,
    #[serde(default)]
    lease_until: u64,
}

impl Task {
    fn snapshot(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("task is serializable");
        for key in ["input", "checkpoint", "resume", "leaseToken", "leaseUntil"] {
            value.as_object_mut().unwrap().remove(key);
        }
        if self.status != "waiting_input" {
            value["result"] = Value::Null;
        } else if let Some(object) = value["result"].as_object_mut() {
            object.retain(|key, _| {
                [
                    "status",
                    "runId",
                    "projectId",
                    "sessionId",
                    "entryId",
                    "interaction",
                    "pendingMessage",
                ]
                .contains(&key.as_str())
            });
        }
        value
    }

    fn detail(&self) -> Value {
        let mut value = self.snapshot();
        value["result"] = self.result.clone();
        value
    }

    fn active(&self) -> bool {
        matches!(
            self.status.as_str(),
            "queued" | "running" | "cancelling" | "waiting_input"
        )
    }
}

pub(crate) struct TaskStore(Mutex<Connection>);

impl TaskStore {
    pub(crate) fn open(root: &Path) -> Result<Self, String> {
        let path = resolve_sandboxed_path(root, ".daily/tasks.sqlite3")?;
        resolve_sandboxed_path(root, ".daily/tasks.sqlite3-wal")?;
        resolve_sandboxed_path(root, ".daily/tasks.sqlite3-shm")?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
        let connection = Connection::open(path).map_err(err)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(err)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS tasks (
               id TEXT PRIMARY KEY, plugin_id TEXT NOT NULL, request_key TEXT NOT NULL,
               data TEXT NOT NULL, summary TEXT NOT NULL, UNIQUE(plugin_id, request_key));
             CREATE TABLE IF NOT EXISTS task_events (
               seq INTEGER PRIMARY KEY AUTOINCREMENT, plugin_id TEXT NOT NULL,
               task_id TEXT NOT NULL, snapshot TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS task_events_owner ON task_events(plugin_id, seq);
             CREATE TABLE IF NOT EXISTS task_operations (
               task_id TEXT NOT NULL, operation_key TEXT NOT NULL, method TEXT NOT NULL,
               payload TEXT NOT NULL, status TEXT NOT NULL, result TEXT, error TEXT,
               attempt INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY(task_id, operation_key));",
            )
            .map_err(err)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(err)?;
        if version > 1 {
            return Err("Task database was created by a newer Daily version.".into());
        }
        let summary_column: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('tasks') WHERE name='summary')",
                [],
                |row| row.get(0),
            )
            .map_err(err)?;
        if !summary_column {
            connection.execute_batch("ALTER TABLE tasks ADD COLUMN summary TEXT NOT NULL DEFAULT '{}'; UPDATE tasks SET summary=json_remove(data,'$.input','$.checkpoint','$.resume','$.leaseToken');").map_err(err)?;
        }
        let attempt_column: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('task_operations') WHERE name='attempt')",[],|row|row.get(0)).map_err(err)?;
        if !attempt_column {
            connection
                .execute_batch(
                    "ALTER TABLE task_operations ADD COLUMN attempt INTEGER NOT NULL DEFAULT 0;",
                )
                .map_err(err)?;
        }
        connection.execute_batch("CREATE INDEX IF NOT EXISTS task_status ON tasks(json_extract(summary,'$.status')); PRAGMA user_version=1;").map_err(err)?;
        let store = Self(Mutex::new(connection));
        store.transaction(|db| {
            for mut task in running(db)? {
                if matches!(task.status.as_str(), "running" | "cancelling") {
                    task=load(db,&task.plugin_id,&task.id)?;
            task.status = if task.status == "cancelling" { "cancelled" } else { "interrupted" }.into();
                    task.error = Some("Task runtime restarted; continue from the saved checkpoint.".into());
                    task.lease_token = None;
                    save(db, &mut task)?;
                }
            }
            db.execute("UPDATE task_operations SET status='interrupted', error='Operation interrupted; verify its outcome before retrying.' WHERE status='running'", []).map_err(err)?;
            Ok(())
        })?;
        Ok(store)
    }

    fn transaction<T>(
        &self,
        action: impl FnOnce(&Connection) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut connection = self.0.lock().map_err(|_| "Task storage is unavailable.")?;
        let tx = connection.transaction().map_err(err)?;
        let result = action(&tx)?;
        tx.commit().map_err(err)?;
        Ok(result)
    }

    pub(crate) fn rpc(&self, plugin: &str, method: &str, payload: Value) -> Result<Value, String> {
        bounded(&payload)?;
        self.transaction(|db| handle_rpc(db, plugin, method, payload))
    }

    /// Starts once, or returns the recorded outcome. Completion is independent of the worker lease.
    pub(crate) fn begin_operation(
        &self,
        plugin: &str,
        payload: &Value,
    ) -> Result<(bool, Value), String> {
        bounded(payload)?;
        self.transaction(|db| {
            let task = load(db, plugin, field(payload, "taskId")?)?;
            check_lease(&task, payload)?;
            if task.status == "cancelling" { return Err("Task was cancelled.".into()); }
            let key = field(payload, "operationKey")?;
            let method = field(payload, "method")?;
            let input = payload.get("payload").cloned().unwrap_or(Value::Null).to_string();
            let previous = db.query_row("SELECT method,payload,status,result,error,attempt FROM task_operations WHERE task_id=? AND operation_key=?", params![task.id,key], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<String>>(4)?,row.get::<_,u32>(5)?))).optional().map_err(err)?;
            if let Some((old_method, old_input, status, result, error, attempt)) = previous {
                if old_method != method || old_input != input { return Err("Task operation key was reused with different parameters.".into()); }
                if status == "failed" && method != "image.generate" && task.attempt > u64::from(attempt) {
                    db.execute("UPDATE task_operations SET status='running',result=NULL,error=NULL,attempt=? WHERE task_id=? AND operation_key=?", params![task.attempt as i64,task.id,key]).map_err(err)?;
                    return Ok((true,json!({"status":"running"})));
                }
                return Ok((false,json!({"status":status,"result":result.map(|value|serde_json::from_str::<Value>(&value)).transpose().map_err(err)?,"error":error})));
            }
            db.execute("INSERT INTO task_operations(task_id,operation_key,method,payload,status,attempt) VALUES(?,?,?,?,'running',?)",params![task.id,key,method,input,task.attempt as i64]).map_err(err)?;
            Ok((true,json!({"status":"running"})))
        })
    }

    pub(crate) fn complete_operation(
        &self,
        id: &str,
        key: &str,
        result: Result<Value, String>,
    ) -> Result<(), String> {
        self.transaction(|db| {
            let result = result.and_then(|value| { bounded(&value)?; Ok(value) });
            let (status,value,error) = match result { Ok(value) => ("completed",Some(value.to_string()),None), Err(error) => ("failed",None,Some(error)) };
            db.execute("UPDATE task_operations SET status=?,result=?,error=? WHERE task_id=? AND operation_key=? AND status='running'",params![status,value,error,id,key]).map_err(err)?;
            Ok(())
        })
    }

    pub(crate) fn has_active(&self) -> Result<bool, String> {
        self.transaction(|db| {
            expire(db)?;
            db.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE json_extract(summary,'$.status') IN ('queued','running','cancelling','waiting_input'))",[],|row|row.get(0)).map_err(err)
        })
    }

    pub(crate) fn workspace_tasks(&self) -> Result<Value, String> {
        self.transaction(|db| {
            expire(db)?;
            let tasks = snapshot_list(list(db, None, None, None)?.into_iter());
            Ok(json!({"tasks":tasks}))
        })
    }

    pub(crate) fn workspace_task(&self, payload: Value) -> Result<Value, String> {
        let id = field(&payload, "taskId")?;
        let plugin = self.transaction(|db| {
            db.query_row("SELECT plugin_id FROM tasks WHERE id=?", [id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(err)
        })?;
        let action = payload
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("get");
        if !["get", "cancel", "retry"].contains(&action) {
            return Err("Unsupported task action.".into());
        }
        self.rpc(&plugin, &format!("tasks.{action}"), payload)
    }
}

fn handle_rpc(
    db: &Connection,
    plugin: &str,
    method: &str,
    payload: Value,
) -> Result<Value, String> {
    expire(db)?;
    match method {
        "tasks.submit" => {
            let kind = field(&payload, "kind")?;
            let key = field(&payload, "requestKey")?;
            let scope = payload.get("scope").and_then(Value::as_str).unwrap_or("");
            if scope.len() > 256 {
                return Err("Task scope is too long.".into());
            }
            let exclusive = payload
                .get("exclusiveKey")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if exclusive.as_ref().is_some_and(|value| value.len() > 256) {
                return Err("Task exclusive key is too long.".into());
            }
            let input = payload.get("input").cloned().unwrap_or(Value::Null);
            let existing: Option<String> = db
                .query_row(
                    "SELECT id FROM tasks WHERE plugin_id=? AND request_key=?",
                    params![plugin, key],
                    |row| row.get(0),
                )
                .optional()
                .map_err(err)?;
            if let Some(id) = existing {
                let task = load(db, plugin, &id)?;
                if task.kind != kind
                    || task.scope != scope
                    || task.input != input
                    || task.exclusive_key != exclusive
                {
                    return Err(
                        "Task request key was already used with different parameters.".into(),
                    );
                }
                return Ok(task.snapshot());
            }
            if exclusive.is_some() && exclusive_active(db, plugin, exclusive.as_deref(), None)? {
                return Err("A task with this exclusive key is already active.".into());
            }
            let now = unix_timestamp();
            let mut task = Task {
                title: payload
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or(kind)
                    .chars()
                    .take(160)
                    .collect(),
                id: format!("task-{}", unique_stamp()),
                plugin_id: plugin.into(),
                kind: kind.into(),
                scope: scope.into(),
                request_key: key.into(),
                exclusive_key: exclusive,
                status: "queued".into(),
                attempt: 0,
                cursor: 0,
                progress: Value::Null,
                result: Value::Null,
                error: None,
                created_at: now,
                updated_at: now,
                input,
                checkpoint: Value::Null,
                resume: Value::Null,
                lease_token: None,
                lease_until: 0,
            };
            save(db, &mut task)?;
            Ok(task.snapshot())
        }
        "tasks.list" => {
            let scope = payload.get("scope").and_then(Value::as_str);
            let kind = payload.get("kind").and_then(Value::as_str);
            let tasks = snapshot_list(list(db, Some(plugin), scope, kind)?.into_iter());
            let cursor: i64 = db
                .query_row(
                    "SELECT COALESCE(MAX(seq),0) FROM task_events WHERE plugin_id=?",
                    [plugin],
                    |row| row.get(0),
                )
                .map_err(err)?;
            Ok(json!({"tasks": tasks, "cursor": cursor}))
        }
        "tasks.events" => {
            let after = payload.get("after").and_then(Value::as_u64).unwrap_or(0);
            let limit = payload
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .clamp(1, 200);
            let oldest: i64 = db
                .query_row(
                    "SELECT COALESCE(MIN(seq),0) FROM task_events WHERE plugin_id=?",
                    [plugin],
                    |row| row.get(0),
                )
                .map_err(err)?;
            if after > 0 && after < oldest.saturating_sub(1) as u64 {
                return Ok(json!({"events":[],"cursor":after,"reset":true}));
            }
            let mut statement = db.prepare("SELECT seq, snapshot FROM task_events WHERE plugin_id=? AND seq>? ORDER BY seq LIMIT ?").map_err(err)?;
            let rows = statement
                .query_map(
                    params![plugin, after.min(i64::MAX as u64) as i64, limit as i64],
                    |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?)),
                )
                .map_err(err)?;
            let mut cursor = after;
            let mut events = Vec::new();
            let mut bytes = 0;
            for row in rows {
                let (seq, snapshot) = row.map_err(err)?;
                bytes += snapshot.len();
                if bytes > 4 * 1024 * 1024 && !events.is_empty() {
                    break;
                }
                cursor = seq;
                events.push(json!({"seq":seq,"task":serde_json::from_str::<Value>(&snapshot).map_err(err)?}));
            }
            Ok(json!({"events":events,"cursor":cursor}))
        }
        "tasks.claim" => {
            let kinds = payload
                .get("kinds")
                .and_then(Value::as_array)
                .ok_or("Task claim requires kinds.")?;
            if kinds.is_empty()
                || kinds.len() > 64
                || kinds
                    .iter()
                    .any(|kind| kind.as_str().is_none_or(|kind| kind.len() > 128))
            {
                return Err("Task kinds are invalid.".into());
            }
            if running(db)?.len() >= MAX_ACTIVE {
                return Ok(Value::Null);
            }
            for mut task in queued(db, plugin)? {
                if task.status != "queued"
                    || !kinds
                        .iter()
                        .any(|kind| kind.as_str() == Some(task.kind.as_str()))
                {
                    continue;
                }
                task = load(db, plugin, &task.id)?;
                task.status = "running".into();
                task.attempt += 1;
                task.lease_token = Some(format!("lease-{}", unique_stamp()));
                task.lease_until = unix_timestamp() + LEASE_SECONDS;
                save(db, &mut task)?;
                return Ok(
                    json!({"task":task.snapshot(),"input":task.input,"checkpoint":task.checkpoint,"resume":task.resume,"leaseToken":task.lease_token}),
                );
            }
            Ok(Value::Null)
        }
        _ => {
            let id = field(&payload, "taskId")?;
            let mut task = load(db, plugin, id)?;
            match method {
                "tasks.get" => return Ok(task.detail()),
                "tasks.cancel" => {
                    if matches!(task.status.as_str(), "running" | "cancelling") {
                        task.status = "cancelling".into();
                    } else if task.active() || task.status == "interrupted" {
                        task.status = "cancelled".into();
                    } else {
                        return Ok(task.snapshot());
                    }
                }
                "tasks.retry" | "tasks.resume" => {
                    let valid = if method == "tasks.resume" {
                        task.status == "waiting_input"
                    } else {
                        matches!(task.status.as_str(), "interrupted" | "failed" | "cancelled")
                    };
                    if !valid {
                        return Err("Task cannot be continued in its current state.".into());
                    }
                    if task.exclusive_key.is_some()
                        && exclusive_active(
                            db,
                            plugin,
                            task.exclusive_key.as_deref(),
                            Some(&task.id),
                        )?
                    {
                        return Err("Another task with this exclusive key is active.".into());
                    }
                    task.status = "queued".into();
                    task.error = None;
                    task.lease_token = None;
                    if method == "tasks.resume" {
                        task.resume = payload.get("input").cloned().unwrap_or(Value::Null);
                    }
                }
                "tasks.heartbeat" | "tasks.update" | "tasks.finish" => {
                    check_lease(&task, &payload)?;
                    if method == "tasks.heartbeat" {
                        task.lease_until = unix_timestamp() + LEASE_SECONDS;
                        // Heartbeats do not create UI events.
                        write_task(db, &task)?;
                        return Ok(task.snapshot());
                    }
                    if let Some(checkpoint) = payload.get("checkpoint") {
                        task.checkpoint = checkpoint.clone();
                    }
                    if let Some(progress) = payload.get("progress") {
                        if progress.to_string().len() > 512 * 1024 {
                            return Err("Task progress exceeds 512 KiB.".into());
                        }
                        task.progress = progress.clone();
                    }
                    if method == "tasks.finish" {
                        let status = field(&payload, "status")?;
                        if !["completed", "failed", "waiting_input", "cancelled"].contains(&status)
                        {
                            return Err("Invalid task completion status.".into());
                        }
                        task.status = if task.status == "cancelling" {
                            "cancelled"
                        } else {
                            status
                        }
                        .into();
                        task.result = payload.get("result").cloned().unwrap_or(Value::Null);
                        task.error = payload
                            .get("error")
                            .and_then(Value::as_str)
                            .map(|error| error.chars().take(4000).collect());
                        task.lease_token = None;
                    }
                }
                _ => return Err(format!("Unknown task method {method}.")),
            }
            save(db, &mut task)?;
            Ok(task.snapshot())
        }
    }
}

fn check_lease(task: &Task, payload: &Value) -> Result<(), String> {
    if !matches!(task.status.as_str(), "running" | "cancelling")
        || task.lease_until <= unix_timestamp()
        || task.lease_token.as_deref() != Some(field(payload, "leaseToken")?)
    {
        return Err("Task lease expired or belongs to another attempt.".into());
    }
    Ok(())
}
fn field<'a>(payload: &'a Value, key: &str) -> Result<&'a str, String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .ok_or_else(|| format!("Task requires a valid {key}."))
}
fn bounded(value: &Value) -> Result<(), String> {
    if value.to_string().len() > MAX_VALUE_BYTES {
        Err("Task data exceeds the 8 MiB limit.".into())
    } else {
        Ok(())
    }
}
fn err(error: impl std::fmt::Display) -> String {
    format!("Task storage: {error}")
}
fn read_summaries(
    db: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Task>, String> {
    let mut statement = db.prepare(sql).map_err(err)?;
    let rows = statement
        .query_map(params, |row| row.get::<_, String>(0))
        .map_err(err)?;
    rows.map(|row| serde_json::from_str(&row.map_err(err)?).map_err(err))
        .collect()
}
fn running(db: &Connection) -> Result<Vec<Task>, String> {
    read_summaries(db,"SELECT summary FROM tasks WHERE json_extract(summary,'$.status') IN ('running','cancelling')",[])
}
fn queued(db: &Connection, plugin: &str) -> Result<Vec<Task>, String> {
    read_summaries(db,"SELECT summary FROM tasks WHERE plugin_id=? AND json_extract(summary,'$.status')='queued' ORDER BY rowid",[plugin])
}
fn list(
    db: &Connection,
    plugin: Option<&str>,
    scope: Option<&str>,
    kind: Option<&str>,
) -> Result<Vec<Task>, String> {
    read_summaries(db,"SELECT summary FROM tasks WHERE (?1 IS NULL OR plugin_id=?1) AND (?2 IS NULL OR json_extract(summary,'$.scope')=?2) AND (?3 IS NULL OR json_extract(summary,'$.kind')=?3) ORDER BY json_extract(summary,'$.status') IN ('queued','running','cancelling','waiting_input') DESC,rowid DESC LIMIT 200",params![plugin,scope,kind])
}
fn exclusive_active(
    db: &Connection,
    plugin: &str,
    key: Option<&str>,
    exclude: Option<&str>,
) -> Result<bool, String> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE plugin_id=?1 AND json_extract(summary,'$.exclusiveKey')=?2 AND (?3 IS NULL OR id!=?3) AND json_extract(summary,'$.status') IN ('queued','running','cancelling','waiting_input'))",params![plugin,key,exclude],|row|row.get(0)).map_err(err)
}
fn load(db: &Connection, plugin: &str, id: &str) -> Result<Task, String> {
    let data: String = db
        .query_row(
            "SELECT data FROM tasks WHERE id=? AND plugin_id=?",
            params![id, plugin],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?
        .ok_or("Task does not exist for this plugin.")?;
    serde_json::from_str(&data).map_err(err)
}
fn write_task(db: &Connection, task: &Task) -> Result<(), String> {
    let mut summary = task.snapshot();
    summary["leaseUntil"] = json!(task.lease_until);
    db.execute("INSERT INTO tasks(id,plugin_id,request_key,data,summary) VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data,summary=excluded.summary",params![task.id,task.plugin_id,task.request_key,serde_json::to_string(task).map_err(err)?,summary.to_string()]).map_err(err)?;
    Ok(())
}
fn save(db: &Connection, task: &mut Task) -> Result<(), String> {
    if serde_json::to_vec(task).map_err(err)?.len() > 12 * 1024 * 1024 {
        return Err("Task input and checkpoint exceed the 12 MiB total limit.".into());
    }
    task.updated_at = unix_timestamp();
    db.execute(
        "INSERT INTO task_events(plugin_id,task_id,snapshot) VALUES(?,?,'{}')",
        params![task.plugin_id, task.id],
    )
    .map_err(err)?;
    task.cursor = db.last_insert_rowid() as u64;
    db.execute(
        "UPDATE task_events SET snapshot=? WHERE seq=?",
        params![task.snapshot().to_string(), task.cursor as i64],
    )
    .map_err(err)?;
    write_task(db, task)?;
    db.execute("DELETE FROM task_events WHERE plugin_id=?1 AND seq < (SELECT seq FROM task_events WHERE plugin_id=?1 ORDER BY seq DESC LIMIT 1 OFFSET 999)",[&task.plugin_id]).map_err(err)?;
    Ok(())
}
fn snapshot_list(tasks: impl Iterator<Item = Task>) -> Vec<Value> {
    let mut values = Vec::new();
    let mut bytes = 0;
    for task in tasks.take(200) {
        let snapshot = task.snapshot();
        bytes += snapshot.to_string().len();
        if bytes > 4 * 1024 * 1024 && !values.is_empty() {
            break;
        }
        values.push(snapshot);
    }
    values
}

fn expire(db: &Connection) -> Result<(), String> {
    for mut task in running(db)? {
        if matches!(task.status.as_str(), "running" | "cancelling")
            && task.lease_until <= unix_timestamp()
        {
            task = load(db, &task.plugin_id, &task.id)?;
            task.status = if task.status == "cancelling" {
                "cancelled"
            } else {
                "interrupted"
            }
            .into();
            task.error = Some("Task worker stopped; continue from the saved checkpoint.".into());
            task.lease_token = None;
            save(db, &mut task)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "daily-task-test-{}-{}",
            unique_stamp(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn submit(store: &TaskStore, key: &str) -> Value {
        store
            .rpc(
                "writer",
                "tasks.submit",
                json!({"kind":"write","scope":"book","requestKey":key,"input":{"chapter":1}}),
            )
            .unwrap()
    }
    fn claim(store: &TaskStore) -> Value {
        store
            .rpc("writer", "tasks.claim", json!({"kinds":["write"]}))
            .unwrap()
    }
    fn auth(claim: &Value) -> Value {
        json!({"taskId":claim["task"]["id"],"leaseToken":claim["leaseToken"]})
    }

    #[test]
    fn submitting_twice_is_idempotent_and_rejects_conflicting_input() {
        let store = TaskStore::open(&root()).unwrap();
        assert_eq!(submit(&store, "one"), submit(&store, "one"));
        assert!(store
            .rpc(
                "writer",
                "tasks.submit",
                json!({"kind":"write","scope":"book","requestKey":"one","input":{}})
            )
            .is_err());
        assert!(store
            .rpc(
                "other",
                "tasks.get",
                json!({"taskId":submit(&store,"one")["id"]})
            )
            .is_err());
    }

    #[test]
    fn page_disconnect_does_not_cancel_task_and_result_is_durable() {
        let root = root();
        let store = TaskStore::open(&root).unwrap();
        let task = submit(&store, "one");
        let claimed = claim(&store);
        let mut finish = auth(&claimed);
        finish["status"] = json!("completed");
        finish["result"] = json!({"text":"finished without a page"});
        store.rpc("writer", "tasks.finish", finish).unwrap();
        drop(store);
        let reopened = TaskStore::open(&root).unwrap();
        let task = reopened
            .rpc("writer", "tasks.get", json!({"taskId":task["id"]}))
            .unwrap();
        assert_eq!(task["status"], "completed");
        assert_eq!(task["result"]["text"], "finished without a page");
        assert!(task.get("leaseToken").is_none());
    }

    #[test]
    fn runtime_restart_preserves_checkpoint_and_fences_the_old_attempt() {
        let root = root();
        let store = TaskStore::open(&root).unwrap();
        let task = submit(&store, "one");
        let old = claim(&store);
        let mut patch = auth(&old);
        patch["checkpoint"] = json!({"completedSteps":["a"]});
        store.rpc("writer", "tasks.update", patch).unwrap();
        drop(store);
        let store = TaskStore::open(&root).unwrap();
        assert_eq!(
            store
                .rpc("writer", "tasks.get", json!({"taskId":task["id"]}))
                .unwrap()["status"],
            "interrupted"
        );
        store
            .rpc("writer", "tasks.retry", json!({"taskId":task["id"]}))
            .unwrap();
        let fresh = claim(&store);
        assert_eq!(fresh["checkpoint"]["completedSteps"], json!(["a"]));
        assert_eq!(fresh["task"]["attempt"], 2);
        assert!(store.rpc("writer", "tasks.heartbeat", auth(&old)).is_err());
    }

    #[test]
    fn snapshot_cursor_catches_every_change_after_attachment() {
        let store = TaskStore::open(&root()).unwrap();
        submit(&store, "one");
        let snapshot = store
            .rpc("writer", "tasks.list", json!({"scope":"book"}))
            .unwrap();
        let claimed = claim(&store);
        let mut update = auth(&claimed);
        update["progress"] = json!("step 2");
        let updated = store.rpc("writer", "tasks.update", update).unwrap();
        let events = store
            .rpc(
                "writer",
                "tasks.events",
                json!({"after":snapshot["cursor"]}),
            )
            .unwrap();
        assert_eq!(events["events"].as_array().unwrap().len(), 2);
        assert_eq!(events["cursor"], updated["cursor"]);
        let again = store
            .rpc("writer", "tasks.events", json!({"after":events["cursor"]}))
            .unwrap();
        assert!(again["events"].as_array().unwrap().is_empty());
    }

    #[test]
    fn operations_complete_after_worker_loss_and_never_execute_twice() {
        let store = TaskStore::open(&root()).unwrap();
        submit(&store, "one");
        let claimed = claim(&store);
        let mut operation = auth(&claimed);
        operation["operationKey"] = json!("generate");
        operation["method"] = json!("image.generate");
        operation["payload"] = json!({"prompt":"x"});
        assert!(store.begin_operation("writer", &operation).unwrap().0);
        assert!(!store.begin_operation("writer", &operation).unwrap().0);
        store
            .complete_operation(
                claimed["task"]["id"].as_str().unwrap(),
                "generate",
                Ok(json!({"file":"result.png"})),
            )
            .unwrap();
        let (start, result) = store.begin_operation("writer", &operation).unwrap();
        assert!(!start);
        assert_eq!(result["result"]["file"], "result.png");
        operation["payload"] = json!({"prompt":"different"});
        assert!(store.begin_operation("writer", &operation).is_err());
    }

    #[test]
    fn uncertain_external_operations_are_not_replayed_after_restart() {
        let root = root();
        let store = TaskStore::open(&root).unwrap();
        let task = submit(&store, "one");
        let old = claim(&store);
        let mut operation = auth(&old);
        operation["operationKey"] = json!("paid-call");
        operation["method"] = json!("image.generate");
        operation["payload"] = json!({});
        store.begin_operation("writer", &operation).unwrap();
        drop(store);
        let store = TaskStore::open(&root).unwrap();
        store
            .rpc("writer", "tasks.retry", json!({"taskId":task["id"]}))
            .unwrap();
        let new = claim(&store);
        operation["leaseToken"] = new["leaseToken"].clone();
        let (start, result) = store.begin_operation("writer", &operation).unwrap();
        assert!(!start);
        assert_eq!(result["status"], "interrupted");
    }

    #[test]
    fn cancellation_is_explicit_and_wins_over_a_late_success() {
        let store = TaskStore::open(&root()).unwrap();
        let task = submit(&store, "one");
        let claimed = claim(&store);
        store
            .rpc("writer", "tasks.cancel", json!({"taskId":task["id"]}))
            .unwrap();
        assert_eq!(
            store
                .rpc("writer", "tasks.heartbeat", auth(&claimed))
                .unwrap()["status"],
            "cancelling"
        );
        let mut finish = auth(&claimed);
        finish["status"] = json!("completed");
        assert_eq!(
            store.rpc("writer", "tasks.finish", finish).unwrap()["status"],
            "cancelled"
        );
    }

    #[test]
    fn waiting_for_input_is_persisted_and_resumes_the_same_task() {
        let root = root();
        let store = TaskStore::open(&root).unwrap();
        let task = submit(&store, "one");
        let claimed = claim(&store);
        let mut finish = auth(&claimed);
        finish["status"] = json!("waiting_input");
        finish["checkpoint"] = json!({"question":"choose"});
        store.rpc("writer", "tasks.finish", finish).unwrap();
        drop(store);
        let store = TaskStore::open(&root).unwrap();
        store
            .rpc(
                "writer",
                "tasks.resume",
                json!({"taskId":task["id"],"input":{"answer":"yes"}}),
            )
            .unwrap();
        let claimed = claim(&store);
        assert_eq!(claimed["resume"]["answer"], "yes");
        assert_eq!(claimed["checkpoint"]["question"], "choose");
    }

    #[test]
    fn expired_workers_cannot_publish_or_finish_an_attempt() {
        let store = TaskStore::open(&root()).unwrap();
        let task = submit(&store, "one");
        let claimed = claim(&store);
        store
            .transaction(|db| {
                let mut task = load(db, "writer", task["id"].as_str().unwrap())?;
                task.lease_until = 0;
                write_task(db, &task)
            })
            .unwrap();
        assert_eq!(
            store
                .rpc("writer", "tasks.get", json!({"taskId":task["id"]}))
                .unwrap()["status"],
            "interrupted"
        );
        let mut finish = auth(&claimed);
        finish["status"] = json!("completed");
        assert!(store.rpc("writer", "tasks.finish", finish).is_err());
    }

    #[test]
    fn compacted_event_history_requires_a_fresh_snapshot() {
        let store = TaskStore::open(&root()).unwrap();
        let task = submit(&store, "one");
        store
            .transaction(|db| {
                let mut task = load(db, "writer", task["id"].as_str().unwrap())?;
                for index in 0..1_010 {
                    task.progress = json!(index);
                    save(db, &mut task)?;
                }
                Ok(())
            })
            .unwrap();
        let old = store
            .rpc("writer", "tasks.events", json!({"after":task["cursor"]}))
            .unwrap();
        assert_eq!(old["reset"], true);
        let snapshot = store.rpc("writer", "tasks.list", json!({})).unwrap();
        assert_eq!(snapshot["tasks"][0]["progress"], 1_009);
        assert!(store
            .rpc(
                "writer",
                "tasks.events",
                json!({"after":snapshot["cursor"]})
            )
            .unwrap()["events"]
            .as_array()
            .unwrap()
            .is_empty());
    }
}
