//! Durable agent task queue records and lifecycle transitions.
//!
//! Task execution is intentionally separate from this CRUD surface. The task
//! record is durable now so a future worker (and the per-task sandbox work in
//! `agentkernel-dse.2`) can claim queued work without changing the API model.

use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::durable_storage::DurableStorage;
use crate::validation;

const MAX_TASK_PROMPT_LEN: usize = 1_048_576;

/// Lifecycle state for an agent task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        })
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(format!("invalid task status '{other}'")),
        }
    }
}

/// Durable task record returned by the REST API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: String,
    pub prompt: String,
    pub sandbox: String,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Result of an atomic task cancellation attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelOutcome {
    NotFound,
    Cancelled(TaskRecord),
    NotCancellable(TaskRecord),
}

/// SQLite-backed task manager.
#[derive(Debug, Clone)]
pub struct TaskManager {
    storage: DurableStorage,
}

impl TaskManager {
    /// Create a manager backed by an already-bootstrapped durable store.
    pub fn new(storage: DurableStorage) -> Self {
        Self { storage }
    }

    /// Open the shared AgentKernel durable database.
    pub fn open_default() -> Result<Self> {
        Ok(Self::new(DurableStorage::open_default()?))
    }

    /// Submit a queued task.
    pub fn create(&self, prompt: &str, sandbox: &str) -> Result<TaskRecord> {
        validate_task_prompt(prompt)?;
        validation::validate_sandbox_name(sandbox)?;

        let id = uuid::Uuid::now_v7().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.storage.open_connection()?;
        conn.execute(
            r#"
INSERT INTO tasks (id, prompt, sandbox, status, result, error, created_at, updated_at)
VALUES (?1, ?2, ?3, 'queued', NULL, NULL, ?4, ?4)
"#,
            params![id, prompt, sandbox, now],
        )
        .context("failed to create task")?;

        Ok(TaskRecord {
            id,
            prompt: prompt.to_string(),
            sandbox: sandbox.to_string(),
            status: TaskStatus::Queued,
            result: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    /// Fetch a task by its UUID.
    pub fn get(&self, id: &str) -> Result<Option<TaskRecord>> {
        let conn = self.storage.open_connection()?;
        conn.query_row(
            "SELECT id, prompt, sandbox, status, result, error, created_at, updated_at FROM tasks WHERE id = ?1",
            [id],
            Self::row_to_task,
        )
        .optional()
        .context("failed to query task by id")
    }

    /// List newest tasks first.
    pub fn list(&self, limit: usize, offset: usize) -> Result<Vec<TaskRecord>> {
        let conn = self.storage.open_connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, prompt, sandbox, status, result, error, created_at, updated_at \
                 FROM tasks ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
            )
            .context("failed to prepare task list query")?;
        let rows = stmt
            .query_map(params![limit as i64, offset as i64], Self::row_to_task)
            .context("failed to execute task list query")?;

        rows.map(|row| row.context("failed to parse task row"))
            .collect()
    }

    /// Atomically cancel queued or running work.
    ///
    /// A conditional update prevents a cancellation racing with a worker's
    /// completion from regressing a terminal task back to `cancelled`.
    pub fn cancel(&self, id: &str) -> Result<CancelOutcome> {
        let mut conn = self.storage.open_connection()?;
        let tx = conn
            .transaction()
            .context("failed to start task cancellation transaction")?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed = tx
            .execute(
                "UPDATE tasks SET status = 'cancelled', updated_at = ?2 WHERE id = ?1 AND status IN ('queued', 'running')",
                params![id, now],
            )
            .context("failed to cancel task")?;

        let current = tx
            .query_row(
                "SELECT id, prompt, sandbox, status, result, error, created_at, updated_at FROM tasks WHERE id = ?1",
                [id],
                Self::row_to_task,
            )
            .optional()
            .context("failed to read task after cancellation")?;
        tx.commit().context("failed to commit task cancellation")?;

        Ok(match current {
            None => CancelOutcome::NotFound,
            Some(task) if changed > 0 || task.status == TaskStatus::Cancelled => {
                CancelOutcome::Cancelled(task)
            }
            Some(task) => CancelOutcome::NotCancellable(task),
        })
    }

    /// Atomically claim a queued task for a worker.
    #[allow(dead_code)]
    pub fn start(&self, id: &str) -> Result<Option<TaskRecord>> {
        self.transition(id, TaskStatus::Queued, TaskStatus::Running, None, None)
    }

    /// Atomically complete a running task.
    #[allow(dead_code)]
    pub fn complete(&self, id: &str, result: &str) -> Result<Option<TaskRecord>> {
        self.transition(
            id,
            TaskStatus::Running,
            TaskStatus::Completed,
            Some(result),
            None,
        )
    }

    /// Atomically fail a queued or running task.
    #[allow(dead_code)]
    pub fn fail(&self, id: &str, error: &str) -> Result<Option<TaskRecord>> {
        let mut conn = self.storage.open_connection()?;
        let tx = conn
            .transaction()
            .context("failed to start task failure transaction")?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed = tx
            .execute(
                "UPDATE tasks SET status = 'failed', error = ?2, updated_at = ?3 WHERE id = ?1 AND status IN ('queued', 'running')",
                params![id, error, now],
            )
            .context("failed to mark task failed")?;
        let current = tx
            .query_row(
                "SELECT id, prompt, sandbox, status, result, error, created_at, updated_at FROM tasks WHERE id = ?1",
                [id],
                Self::row_to_task,
            )
            .optional()
            .context("failed to read task after failure")?;
        tx.commit().context("failed to commit task failure")?;
        Ok((changed > 0).then_some(current).flatten())
    }

    #[allow(dead_code)]
    fn transition(
        &self,
        id: &str,
        from: TaskStatus,
        to: TaskStatus,
        result: Option<&str>,
        error: Option<&str>,
    ) -> Result<Option<TaskRecord>> {
        let mut conn = self.storage.open_connection()?;
        let tx = conn
            .transaction()
            .context("failed to start task transition transaction")?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed = tx
            .execute(
                "UPDATE tasks SET status = ?2, result = ?3, error = ?4, updated_at = ?5 WHERE id = ?1 AND status = ?6",
                params![id, to.to_string(), result, error, now, from.to_string()],
            )
            .context("failed to transition task")?;
        let current = if changed > 0 {
            tx.query_row(
                "SELECT id, prompt, sandbox, status, result, error, created_at, updated_at FROM tasks WHERE id = ?1",
                [id],
                Self::row_to_task,
            )
            .optional()
            .context("failed to read task after transition")?
        } else {
            None
        };
        tx.commit().context("failed to commit task transition")?;
        Ok(current)
    }

    fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
        let status_raw: String = row.get(3)?;
        let status = status_raw.parse::<TaskStatus>().map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
            )
        })?;
        Ok(TaskRecord {
            id: row.get(0)?,
            prompt: row.get(1)?,
            sandbox: row.get(2)?,
            status,
            result: row.get(4)?,
            error: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }
}

/// Validate an API task ID before querying SQLite.
pub fn validate_task_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| anyhow::anyhow!("invalid task ID"))
}

/// Validate the user prompt before storing or dispatching it.
pub fn validate_task_prompt(prompt: &str) -> Result<()> {
    if prompt.trim().is_empty() {
        bail!("prompt is required");
    }
    if prompt.len() > MAX_TASK_PROMPT_LEN {
        bail!("prompt is too long (max {} bytes)", MAX_TASK_PROMPT_LEN);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn manager() -> (tempfile::TempDir, TaskManager) {
        let temp = tempfile::TempDir::new().unwrap();
        let storage = DurableStorage::new(temp.path().join("tasks.db")).unwrap();
        (temp, TaskManager::new(storage))
    }

    #[test]
    fn task_lifecycle_is_persisted_and_atomic() {
        let (_temp, manager) = manager();
        let task = manager.create("fix the tests", "sandbox-1").unwrap();
        assert_eq!(task.status, TaskStatus::Queued);
        assert_eq!(
            manager.start(&task.id).unwrap().unwrap().status,
            TaskStatus::Running
        );
        assert_eq!(
            manager
                .complete(&task.id, "all fixed")
                .unwrap()
                .unwrap()
                .status,
            TaskStatus::Completed
        );
        assert!(manager.start(&task.id).unwrap().is_none());
        assert!(matches!(
            manager.cancel(&task.id).unwrap(),
            CancelOutcome::NotCancellable(record) if record.status == TaskStatus::Completed
        ));

        let failed = manager.create("fail this task", "sandbox-1").unwrap();
        manager.start(&failed.id).unwrap();
        let failed = manager.fail(&failed.id, "worker exited").unwrap().unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("worker exited"));

        let running = manager.create("cancel while running", "sandbox-1").unwrap();
        manager.start(&running.id).unwrap();
        assert!(matches!(
            manager.cancel(&running.id).unwrap(),
            CancelOutcome::Cancelled(record) if record.status == TaskStatus::Cancelled
        ));

        let listed = manager.list(10, 0).unwrap();
        assert_eq!(listed.len(), 3);
    }

    #[test]
    fn cancellation_is_idempotent_for_cancelled_tasks() {
        let (_temp, manager) = manager();
        let task = manager.create("stop this", "sandbox-1").unwrap();
        assert!(matches!(
            manager.cancel(&task.id).unwrap(),
            CancelOutcome::Cancelled(record) if record.status == TaskStatus::Cancelled
        ));
        assert!(matches!(
            manager.cancel(&task.id).unwrap(),
            CancelOutcome::Cancelled(record) if record.status == TaskStatus::Cancelled
        ));
    }

    #[test]
    fn concurrent_cancellation_is_atomic() {
        let (_temp, manager) = manager();
        let task = manager.create("cancel concurrently", "sandbox-1").unwrap();
        let manager = Arc::new(manager);
        let handles = (0..8)
            .map(|_| {
                let manager = Arc::clone(&manager);
                let task_id = task.id.clone();
                std::thread::spawn(move || manager.cancel(&task_id).unwrap())
            })
            .collect::<Vec<_>>();

        for handle in handles {
            assert!(matches!(
                handle.join().unwrap(),
                CancelOutcome::Cancelled(_)
            ));
        }
        assert_eq!(
            manager.get(&task.id).unwrap().unwrap().status,
            TaskStatus::Cancelled
        );
    }

    #[test]
    fn validation_rejects_empty_or_invalid_inputs() {
        assert!(validate_task_prompt(" ").is_err());
        assert!(validate_task_prompt(&"x".repeat(MAX_TASK_PROMPT_LEN + 1)).is_err());
        assert!(validate_task_id("not-a-uuid").is_err());
        assert!(validation::validate_sandbox_name("bad/name").is_err());
    }
}
