//! Single-task worker and per-task sandbox/Git isolation.
//!
//! This module deliberately owns one task at a time.  The later parallel
//! coordinator can compose the worker without changing claiming or cleanup
//! semantics.

use anyhow::Result;
use async_trait::async_trait;
use tokio::time::{Duration, interval, sleep};

use crate::tasks::{TaskIsolation, TaskManager, TaskRecord, TaskStatus};

const TASK_LEASE_SECONDS: i64 = 300;
const TASK_HEARTBEAT_SECONDS: u64 = 5;

/// Execution boundary used by the durable worker.
///
/// Keeping provisioning behind this trait makes failures and cancellation
/// races testable without requiring Docker, KVM, or an installed agent CLI.
#[async_trait]
pub trait TaskExecutor: Send {
    /// Allocate the worktree and sandbox. Implementations must roll back any
    /// partial allocation before returning an error, because the worker cannot
    /// safely assume that a pre-existing resource belongs to the task.
    async fn prepare(
        &mut self,
        task: &TaskRecord,
        planned: &TaskIsolation,
    ) -> Result<TaskIsolation>;

    /// Run the agent and return a reviewable result (normally a Git diff).
    async fn execute(&mut self, task: &TaskRecord, isolation: &TaskIsolation) -> Result<String>;

    /// Release runtime/worktree resources. This is called on success, failure,
    /// and cancellation, and should be idempotent.
    async fn cleanup(&mut self, task: &TaskRecord, isolation: &TaskIsolation) -> Result<()>;
}

/// Durable one-at-a-time task worker.
#[derive(Debug)]
pub struct TaskWorker<E> {
    manager: TaskManager,
    executor: E,
    worker_id: String,
}

impl<E> TaskWorker<E>
where
    E: TaskExecutor,
{
    pub fn new(manager: TaskManager, executor: E) -> Self {
        Self::with_worker_id(manager, executor, uuid::Uuid::now_v7().to_string())
    }

    /// Construct a worker with a caller-supplied identity. Coordinators use
    /// this to register ownership before spawning worker futures.
    pub fn with_worker_id(manager: TaskManager, executor: E, worker_id: String) -> Self {
        Self {
            manager,
            executor,
            worker_id,
        }
    }

    fn lease_deadline() -> String {
        (chrono::Utc::now() + chrono::Duration::seconds(TASK_LEASE_SECONDS))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    /// Fail tasks orphaned by a previous worker process and clean up their
    /// deterministic sandbox/checkout locations before accepting new work.
    pub async fn recover_interrupted(&mut self) -> Result<usize> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let expired = self.manager.expired_running(&now)?;
        let mut recovered = 0;
        for candidate in expired {
            if let Some(task) = self.manager.fail_expired(
                &candidate.id,
                &now,
                "task worker lease expired before completion",
            )? {
                let isolation = task
                    .isolation
                    .clone()
                    .unwrap_or_else(|| TaskIsolation::planned(&task.id));
                if let Err(error) = self.executor.cleanup(&task, &isolation).await {
                    eprintln!("[tasks] recovery cleanup for {} failed: {error:#}", task.id);
                }
                recovered += 1;
            }
        }
        Ok(recovered)
    }

    /// Claim and process the oldest queued task, if one is available.
    pub async fn run_once(&mut self) -> Result<Option<TaskRecord>> {
        let Some(candidate) = self.manager.next_queued()? else {
            return Ok(None);
        };

        self.run_task(&candidate.id).await
    }

    /// Claim and process exactly `id` if it is still queued. Unlike
    /// `run_once`, this never searches for another task, which lets a
    /// coordinator operate on a fixed snapshot and leave tasks submitted
    /// after that snapshot to a later run.
    pub async fn run_task(&mut self, id: &str) -> Result<Option<TaskRecord>> {
        let Some(candidate) = self.manager.get(id)? else {
            return Ok(None);
        };
        if candidate.status != TaskStatus::Queued {
            return Ok(None);
        }

        let planned = TaskIsolation::planned(&candidate.id);
        let Some(claimed) = self.manager.claim_with_isolation(
            &candidate.id,
            &planned,
            &self.worker_id,
            &Self::lease_deadline(),
        )?
        else {
            // Another worker won the conditional update. The next tick will
            // select another queued task.
            return Ok(None);
        };

        let prepared = self.executor.prepare(&claimed, &planned).await;
        let isolation = match prepared {
            Ok(isolation) => {
                // If cancellation won while provisioning, persist nothing
                // further and immediately clean up the partial allocation.
                let persisted = match self.manager.update_isolation(&claimed.id, &isolation) {
                    Ok(persisted) => persisted,
                    Err(error) => {
                        let _ = self.executor.cleanup(&claimed, &isolation).await;
                        return Err(error);
                    }
                };
                if persisted.is_none() {
                    let _ = self.executor.cleanup(&claimed, &isolation).await;
                    return self.manager.get(&claimed.id);
                }
                isolation
            }
            Err(error) => {
                let _ = self.manager.fail(&claimed.id, &error.to_string());
                return self.manager.get(&claimed.id);
            }
        };

        if self
            .manager
            .get(&claimed.id)?
            .is_some_and(|task| task.status == TaskStatus::Cancelled)
        {
            let _ = self.executor.cleanup(&claimed, &isolation).await;
            return self.manager.get(&claimed.id);
        }

        let execution = {
            let execution = self.executor.execute(&claimed, &isolation);
            tokio::pin!(execution);
            let mut heartbeat = interval(Duration::from_secs(TASK_HEARTBEAT_SECONDS));
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    result = &mut execution => break Some(result),
                    _ = sleep(Duration::from_millis(250)) => {
                        if self.manager.get(&claimed.id)?.is_some_and(|task| task.status == TaskStatus::Cancelled) {
                            break None;
                        }
                    }
                    _ = heartbeat.tick() => {
                        if !self.manager.renew_lease(&claimed.id, &self.worker_id, &Self::lease_deadline())? {
                            break None;
                        }
                    }
                }
            }
        };
        let Some(execution) = execution else {
            let _ = self.executor.cleanup(&claimed, &isolation).await;
            return self.manager.get(&claimed.id);
        };
        let final_task = match execution {
            Ok(result) => match self.manager.complete(&claimed.id, &result) {
                Ok(task) => task,
                Err(error) => {
                    let _ = self.executor.cleanup(&claimed, &isolation).await;
                    return Err(error);
                }
            },
            Err(error) => match self.manager.fail(&claimed.id, &error.to_string()) {
                Ok(task) => task,
                Err(storage_error) => {
                    let _ = self.executor.cleanup(&claimed, &isolation).await;
                    return Err(storage_error);
                }
            },
        };
        if let Err(error) = self.executor.cleanup(&claimed, &isolation).await {
            // Cleanup cannot safely regress a completed/failed/cancelled task;
            // surface it for operators while preserving the agent result.
            eprintln!("[tasks] cleanup for {} failed: {error:#}", claimed.id);
        }

        Ok(final_task.or(self.manager.get(&claimed.id)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_storage::DurableStorage;
    use anyhow::bail;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    #[derive(Debug, Clone)]
    struct FakeExecutor {
        prepared: Arc<AtomicUsize>,
        executed: Arc<AtomicUsize>,
        cleaned: Arc<AtomicUsize>,
        fail_prepare: bool,
        fail_execute: bool,
    }

    #[async_trait]
    impl TaskExecutor for FakeExecutor {
        async fn prepare(
            &mut self,
            _task: &TaskRecord,
            planned: &TaskIsolation,
        ) -> Result<TaskIsolation> {
            self.prepared.fetch_add(1, Ordering::SeqCst);
            if self.fail_prepare {
                bail!("provision failed");
            }
            Ok(TaskIsolation {
                worktree: Some("/tmp/task-worktree".to_string()),
                base_ref: Some("abc123".to_string()),
                ..planned.clone()
            })
        }

        async fn execute(
            &mut self,
            _task: &TaskRecord,
            _isolation: &TaskIsolation,
        ) -> Result<String> {
            self.executed.fetch_add(1, Ordering::SeqCst);
            if self.fail_execute {
                bail!("agent failed");
            }
            Ok("diff --git a/file b/file".to_string())
        }

        async fn cleanup(&mut self, _task: &TaskRecord, _isolation: &TaskIsolation) -> Result<()> {
            self.cleaned.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn fixture() -> (TempDir, TaskManager) {
        let temp = TempDir::new().unwrap();
        let manager = TaskManager::new(DurableStorage::new(temp.path().join("tasks.db")).unwrap());
        (temp, manager)
    }

    #[tokio::test]
    async fn worker_claims_persists_diff_and_cleans_up() {
        let (_temp, manager) = fixture();
        let _task = manager.create("make a change", "template").unwrap();
        let executor = FakeExecutor {
            prepared: Arc::new(AtomicUsize::new(0)),
            executed: Arc::new(AtomicUsize::new(0)),
            cleaned: Arc::new(AtomicUsize::new(0)),
            fail_prepare: false,
            fail_execute: false,
        };
        let cleaned = executor.cleaned.clone();
        let mut worker = TaskWorker::new(manager.clone(), executor);
        let result = worker.run_once().await.unwrap().unwrap();
        assert_eq!(result.status, TaskStatus::Completed);
        assert_eq!(result.result.as_deref(), Some("diff --git a/file b/file"));
        assert_eq!(
            result.isolation.unwrap().base_ref.as_deref(),
            Some("abc123")
        );
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn worker_persists_failure_and_still_cleans_up() {
        let (_temp, manager) = fixture();
        let _task = manager.create("make a change", "template").unwrap();
        let executor = FakeExecutor {
            prepared: Arc::new(AtomicUsize::new(0)),
            executed: Arc::new(AtomicUsize::new(0)),
            cleaned: Arc::new(AtomicUsize::new(0)),
            fail_prepare: false,
            fail_execute: true,
        };
        let cleaned = executor.cleaned.clone();
        let mut worker = TaskWorker::new(manager.clone(), executor);
        let result = worker.run_once().await.unwrap().unwrap();
        assert_eq!(result.status, TaskStatus::Failed);
        assert_eq!(result.error.as_deref(), Some("agent failed"));
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[derive(Debug, Clone)]
    struct CancellingExecutor {
        manager: TaskManager,
        cleaned: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl TaskExecutor for CancellingExecutor {
        async fn prepare(
            &mut self,
            task: &TaskRecord,
            planned: &TaskIsolation,
        ) -> Result<TaskIsolation> {
            let _ = self.manager.cancel(&task.id)?;
            Ok(planned.clone())
        }

        async fn execute(
            &mut self,
            _task: &TaskRecord,
            _isolation: &TaskIsolation,
        ) -> Result<String> {
            bail!("cancelled task must not execute")
        }

        async fn cleanup(&mut self, _task: &TaskRecord, _isolation: &TaskIsolation) -> Result<()> {
            self.cleaned.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancellation_during_prepare_skips_execution_and_cleans_up() {
        let (_temp, manager) = fixture();
        let task = manager.create("cancel me", "template").unwrap();
        let cleaned = Arc::new(AtomicUsize::new(0));
        let mut worker = TaskWorker::new(
            manager.clone(),
            CancellingExecutor {
                manager: manager.clone(),
                cleaned: cleaned.clone(),
            },
        );

        let result = worker.run_once().await.unwrap().unwrap();
        assert_eq!(result.id, task.id);
        assert_eq!(result.status, TaskStatus::Cancelled);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_workers_only_execute_once() {
        let (_temp, manager) = fixture();
        let task = manager.create("make a change", "template").unwrap();
        let manager_a = manager.clone();
        let manager_b = manager.clone();
        let a = tokio::spawn(async move {
            let executor = FakeExecutor {
                prepared: Arc::new(AtomicUsize::new(0)),
                executed: Arc::new(AtomicUsize::new(0)),
                cleaned: Arc::new(AtomicUsize::new(0)),
                fail_prepare: false,
                fail_execute: false,
            };
            TaskWorker::new(manager_a, executor)
                .run_once()
                .await
                .unwrap()
        });
        let b = tokio::spawn(async move {
            let executor = FakeExecutor {
                prepared: Arc::new(AtomicUsize::new(0)),
                executed: Arc::new(AtomicUsize::new(0)),
                cleaned: Arc::new(AtomicUsize::new(0)),
                fail_prepare: false,
                fail_execute: false,
            };
            TaskWorker::new(manager_b, executor)
                .run_once()
                .await
                .unwrap()
        });
        let (a, b) = tokio::join!(a, b);
        let processed = [a.unwrap(), b.unwrap()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(processed.len(), 1);
        assert_eq!(processed[0].id, task.id);
        assert_eq!(
            manager.get(&task.id).unwrap().unwrap().status,
            TaskStatus::Completed
        );
    }

    #[derive(Debug)]
    struct SlowExecutor {
        cleaned: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl TaskExecutor for SlowExecutor {
        async fn prepare(
            &mut self,
            _task: &TaskRecord,
            planned: &TaskIsolation,
        ) -> Result<TaskIsolation> {
            Ok(planned.clone())
        }

        async fn execute(
            &mut self,
            _task: &TaskRecord,
            _isolation: &TaskIsolation,
        ) -> Result<String> {
            sleep(Duration::from_secs(30)).await;
            Ok("too late".to_string())
        }

        async fn cleanup(&mut self, _task: &TaskRecord, _isolation: &TaskIsolation) -> Result<()> {
            self.cleaned.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn cancellation_during_execution_stops_waiting_and_cleans_up() {
        let (_temp, manager) = fixture();
        let task = manager.create("cancel execution", "template").unwrap();
        let cancel_manager = manager.clone();
        let task_id = task.id.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(20)).await;
            cancel_manager.cancel(&task_id).unwrap();
        });
        let cleaned = Arc::new(AtomicUsize::new(0));
        let mut worker = TaskWorker::new(
            manager,
            SlowExecutor {
                cleaned: cleaned.clone(),
            },
        );

        let result = worker.run_once().await.unwrap().unwrap();
        assert_eq!(result.status, TaskStatus::Cancelled);
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn recovery_fails_orphaned_running_tasks_and_cleans_up() {
        let (_temp, manager) = fixture();
        let task = manager.create("interrupted", "template").unwrap();
        manager.start(&task.id).unwrap();
        let cleaned = Arc::new(AtomicUsize::new(0));
        let executor = FakeExecutor {
            prepared: Arc::new(AtomicUsize::new(0)),
            executed: Arc::new(AtomicUsize::new(0)),
            cleaned: cleaned.clone(),
            fail_prepare: false,
            fail_execute: false,
        };
        let mut worker = TaskWorker::new(manager.clone(), executor);

        assert_eq!(worker.recover_interrupted().await.unwrap(), 1);
        let recovered = manager.get(&task.id).unwrap().unwrap();
        assert_eq!(recovered.status, TaskStatus::Failed);
        assert_eq!(
            recovered.error.as_deref(),
            Some("task worker lease expired before completion")
        );
        assert_eq!(cleaned.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn recovery_does_not_touch_a_live_worker_lease() {
        let (_temp, manager) = fixture();
        let task = manager.create("still running", "template").unwrap();
        let planned = TaskIsolation::planned(&task.id);
        manager
            .claim_with_isolation(
                &task.id,
                &planned,
                "live-worker",
                &(chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
            )
            .unwrap();
        let cleaned = Arc::new(AtomicUsize::new(0));
        let executor = FakeExecutor {
            prepared: Arc::new(AtomicUsize::new(0)),
            executed: Arc::new(AtomicUsize::new(0)),
            cleaned: cleaned.clone(),
            fail_prepare: false,
            fail_execute: false,
        };
        let mut worker = TaskWorker::new(manager.clone(), executor);

        assert_eq!(worker.recover_interrupted().await.unwrap(), 0);
        assert_eq!(
            manager.get(&task.id).unwrap().unwrap().status,
            TaskStatus::Running
        );
        assert_eq!(cleaned.load(Ordering::SeqCst), 0);
    }
}
