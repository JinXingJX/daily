//! Durable storage for plugin agent runs paused on an async tool.
//!
//! `agent.run` and `agent.task.*` persist different run payloads but share the
//! same bounded, atomic, per-run JSON file layout under `.daily/`.

use crate::fs_utils::atomic_write;
use crate::fs_utils::sandbox::read_limited_text;
use crate::limits::MAX_PENDING_PLUGIN_RUN_BYTES;
use crate::plugins::rpc::storage::sanitize_plugin_file_name;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// One directory of pending runs of a single payload type.
pub(crate) struct PendingRunStore {
    dir_name: &'static str,
    label: &'static str,
}

/// Runs started with `agent.run`.
pub(crate) const AGENT_RUNS: PendingRunStore = PendingRunStore {
    dir_name: "plugin-agent-runs",
    label: "plugin agent run",
};

/// Runs started with `agent.task.start`.
pub(crate) const AGENT_TASK_RUNS: PendingRunStore = PendingRunStore {
    dir_name: "plugin-agent-task-runs",
    label: "plugin agent task run",
};

impl PendingRunStore {
    pub(crate) fn path(&self, project_root: &Path, run_id: &str) -> PathBuf {
        project_root
            .join(".daily")
            .join(self.dir_name)
            .join(format!("{}.json", sanitize_plugin_file_name(run_id)))
    }

    pub(crate) fn persist<T: Serialize>(
        &self,
        project_root: &Path,
        run_id: &str,
        pending: &T,
    ) -> Result<(), String> {
        let label = self.label;
        let path = self.path(project_root, run_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create {label} directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let payload = serde_json::to_string_pretty(pending)
            .map_err(|error| format!("Failed to serialize pending {label}: {error}"))?;
        if payload.len() > MAX_PENDING_PLUGIN_RUN_BYTES {
            return Err(format!(
                "Pending {label} exceeds the {MAX_PENDING_PLUGIN_RUN_BYTES} byte limit."
            ));
        }
        atomic_write(&path, format!("{payload}\n")).map_err(|error| {
            format!(
                "Failed to write pending {label} {}: {error}",
                path.display()
            )
        })
    }

    pub(crate) fn load<T: DeserializeOwned>(
        &self,
        project_root: &Path,
        run_id: &str,
    ) -> Result<T, String> {
        let label = self.label;
        let path = self.path(project_root, run_id);
        let raw = read_limited_text(
            &path,
            MAX_PENDING_PLUGIN_RUN_BYTES,
            &format!("pending {label}"),
        )?;
        serde_json::from_str(&raw).map_err(|error| {
            format!(
                "Failed to parse pending {label} {}: {error}",
                path.display()
            )
        })
    }

    pub(crate) fn remove(&self, project_root: &Path, run_id: &str) -> Result<(), String> {
        let path = self.path(project_root, run_id);
        if path.exists() {
            fs::remove_file(&path).map_err(|error| {
                format!(
                    "Failed to remove pending {} {}: {error}",
                    self.label,
                    path.display()
                )
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{AGENT_RUNS, AGENT_TASK_RUNS};
    use crate::util::unique_stamp;
    use serde_json::{json, Value};

    #[test]
    fn stores_keep_separate_directories_and_round_trip() {
        let root = std::env::temp_dir().join(format!("daily-pending-runs-{}", unique_stamp()));
        let run = json!({ "runId": "run-1", "createdAt": 1 });

        AGENT_RUNS.persist(&root, "run-1", &run).unwrap();
        assert!(root.join(".daily/plugin-agent-runs/run-1.json").exists());
        assert!(AGENT_TASK_RUNS.load::<Value>(&root, "run-1").is_err());
        assert_eq!(AGENT_RUNS.load::<Value>(&root, "run-1").unwrap(), run);

        AGENT_RUNS.remove(&root, "run-1").unwrap();
        assert!(AGENT_RUNS.load::<Value>(&root, "run-1").is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
