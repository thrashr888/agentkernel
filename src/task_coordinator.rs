//! Bounded parallel execution for the durable task queue.
//!
//! The coordinator deliberately composes [`TaskWorker`] instead of
//! reimplementing claiming, isolation, cancellation, or cleanup. Each worker
//! gets its own executor instance, while `TaskManager`'s conditional claim
//! keeps multiple coordinator workers from processing one task twice.

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinSet;
use tokio::time::{Duration, sleep};

use crate::task_worker::{TaskExecutor, TaskWorker};
use crate::tasks::{TaskManager, TaskRecord, TaskStatus};

/// A point-in-time view of the tasks owned by one coordinator run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskProgress {
    pub total: usize,
    pub queued: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub skipped: usize,
}

impl TaskProgress {
    pub fn finished(self) -> usize {
        self.completed + self.failed + self.cancelled + self.skipped
    }
}

/// Aggregated terminal records produced by a coordinator run.
#[derive(Debug, Clone, Default)]
pub struct TaskRunSummary {
    pub recovered: usize,
    pub cancelled_by_user: bool,
    /// Tasks in the initial snapshot that this coordinator did not own,
    /// normally because another process claimed them first.
    pub skipped: usize,
    pub skipped_ids: Vec<String>,
    pub tasks: Vec<TaskRecord>,
}

impl TaskRunSummary {
    pub fn progress(&self) -> TaskProgress {
        let skipped_ids = self.skipped_ids.iter().cloned().collect::<HashSet<_>>();
        let mut progress = progress_for(&self.tasks, &skipped_ids);
        progress.skipped = self.skipped;
        progress
    }

    pub fn completed(&self) -> usize {
        self.progress().completed
    }

    pub fn failed(&self) -> usize {
        self.progress().failed
    }

    pub fn cancelled(&self) -> usize {
        self.progress().cancelled
    }

    pub fn skipped(&self) -> usize {
        self.skipped
    }
}

/// Runs queued tasks with a fixed upper bound on active workers.
pub struct TaskCoordinator<E, F> {
    manager: TaskManager,
    max_concurrency: usize,
    executor_factory: Arc<F>,
    cancelled: Arc<AtomicBool>,
    _executor: std::marker::PhantomData<fn() -> E>,
}

impl<E, F> TaskCoordinator<E, F>
where
    E: TaskExecutor + 'static,
    F: Fn() -> Result<E> + Send + Sync + 'static,
{
    /// Construct a coordinator. The factory is called once per worker, so
    /// executors must not share mutable runtime state that would serialize
    /// otherwise independent tasks.
    pub fn new(manager: TaskManager, max_concurrency: usize, executor_factory: F) -> Result<Self> {
        if max_concurrency == 0 {
            bail!("task parallelism must be at least 1");
        }
        Ok(Self {
            manager,
            max_concurrency,
            executor_factory: Arc::new(executor_factory),
            cancelled: Arc::new(AtomicBool::new(false)),
            _executor: std::marker::PhantomData,
        })
    }

    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Request a graceful stop. Workers stop claiming queued tasks and the
    /// coordinator cancels tasks already claimed by this run. A running
    /// `TaskWorker` observes that durable state and performs its normal
    /// cleanup path before exiting.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Run the queue and return an aggregate of the tasks that were present
    /// when this run started. No task is claimed more than once, even when
    /// several workers race on the same oldest queued record.
    #[allow(dead_code)]
    pub async fn run_until_idle(&self) -> Result<TaskRunSummary> {
        self.run_with_progress(|_| {}).await
    }

    /// Run the queue while reporting snapshots after each task reaches a
    /// terminal state. The callback runs on the coordinator task, making it
    /// suitable for a CLI progress line without requiring a shared lock.
    pub async fn run_with_progress<P>(&self, mut on_progress: P) -> Result<TaskRunSummary>
    where
        P: FnMut(TaskProgress) + Send,
    {
        let initial = self.manager.list(1_000_000_000, 0)?;
        let task_ids: Arc<Vec<String>> = Arc::new(
            initial
                .iter()
                .filter(|task| matches!(task.status, TaskStatus::Queued | TaskStatus::Running))
                .map(|task| task.id.clone())
                .collect(),
        );
        let queued_ids: Vec<String> = initial
            .iter()
            .filter(|task| task.status == TaskStatus::Queued)
            .map(|task| task.id.clone())
            .collect();

        if task_ids.is_empty() {
            return Ok(TaskRunSummary::default());
        }

        let mut recovered = 0;
        if initial
            .iter()
            .any(|task| task.status == TaskStatus::Running)
        {
            let executor = (self.executor_factory)()
                .map_err(|error| anyhow!("failed to initialize recovery worker: {error:#}"))?;
            let mut worker = TaskWorker::new(self.manager.clone(), executor);
            recovered = worker.recover_interrupted().await?;
        }

        // Running tasks are not ours to claim. They are included in the
        // snapshot and reported as skipped unless recovery just handled an
        // expired lease; this keeps a second coordinator from looking like it
        // completed work it never owned.
        let mut skipped_ids: HashSet<String> = initial
            .iter()
            .filter(|task| task.status == TaskStatus::Running)
            .filter_map(|task| {
                let current = self.manager.get(&task.id).ok().flatten()?;
                let recovered_task = current.status == TaskStatus::Failed
                    && current.error.as_deref()
                        == Some("task worker lease expired before completion");
                (!recovered_task).then_some(task.id.clone())
            })
            .collect();

        let worker_count = self.max_concurrency.min(queued_ids.len());
        let queue = Arc::new(Mutex::new(VecDeque::from(queued_ids)));
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();
        let mut workers = JoinSet::new();
        let mut worker_ids = Vec::with_capacity(worker_count);
        let mut worker_specs = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let worker_id = uuid::Uuid::now_v7().to_string();
            let executor = (self.executor_factory)()
                .map_err(|error| anyhow!("failed to initialize task worker: {error:#}"))?;
            let worker =
                TaskWorker::with_worker_id(self.manager.clone(), executor, worker_id.clone());
            worker_ids.push(worker_id);
            worker_specs.push(worker);
        }
        let worker_ids = Arc::new(worker_ids);
        for worker in worker_specs {
            let queue = queue.clone();
            let cancelled = self.cancelled.clone();
            let progress_tx = progress_tx.clone();
            workers.spawn(async move { worker_loop(worker, queue, cancelled, progress_tx).await });
        }
        drop(progress_tx);

        let monitor = self.spawn_cancellation_monitor(task_ids.clone(), worker_ids.clone());
        let mut worker_error = None;
        while !workers.is_empty() {
            tokio::select! {
                Some(event) = progress_rx.recv() => {
                    match event {
                        WorkerEvent::Skipped(id) => {
                            skipped_ids.insert(id);
                        }
                        WorkerEvent::Processed => {}
                    }
                    on_progress(self.progress_for_ids(&task_ids, &skipped_ids)?);
                }
                Some(joined) = workers.join_next() => {
                    match joined {
                        Err(error) => {
                            self.cancel();
                            worker_error.get_or_insert_with(|| anyhow!("task worker panicked: {error}"));
                        }
                        Ok(Err(error)) => {
                            self.cancel();
                            worker_error.get_or_insert(error);
                        }
                        Ok(Ok(())) => {}
                    }
                }
            }
        }
        if self.is_cancelled() {
            // The monitor normally performs this while a worker is observing
            // cancellation. Repeat after all workers have joined so a very
            // short task cannot leave later queued records behind merely
            // because the monitor's polling interval had not elapsed.
            for id in task_ids.iter() {
                self.manager.cancel_queued(id)?;
            }
            for worker_id in worker_ids.iter() {
                self.manager.cancel_owned_running(worker_id)?;
            }
        }
        monitor.abort();
        while let Ok(event) = progress_rx.try_recv() {
            match event {
                WorkerEvent::Skipped(id) => {
                    skipped_ids.insert(id);
                }
                WorkerEvent::Processed => {}
            }
            on_progress(self.progress_for_ids(&task_ids, &skipped_ids)?);
        }

        if let Some(error) = worker_error {
            return Err(error);
        }

        let mut tasks = Vec::with_capacity(task_ids.len());
        for id in task_ids.iter() {
            if let Some(task) = self.manager.get(id)? {
                tasks.push(task);
            }
        }
        tasks.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(TaskRunSummary {
            recovered,
            cancelled_by_user: self.is_cancelled(),
            skipped: skipped_ids.len(),
            skipped_ids: {
                let mut ids = skipped_ids.into_iter().collect::<Vec<_>>();
                ids.sort();
                ids
            },
            tasks,
        })
    }

    fn progress_for_ids(
        &self,
        task_ids: &[String],
        skipped_ids: &HashSet<String>,
    ) -> Result<TaskProgress> {
        let mut tasks = Vec::with_capacity(task_ids.len());
        for id in task_ids {
            if let Some(task) = self.manager.get(id)? {
                tasks.push(task);
            }
        }
        let mut progress = progress_for(&tasks, skipped_ids);
        progress.skipped = skipped_ids.len();
        Ok(progress)
    }

    fn spawn_cancellation_monitor(
        &self,
        task_ids: Arc<Vec<String>>,
        worker_ids: Arc<Vec<String>>,
    ) -> tokio::task::JoinHandle<()> {
        let manager = self.manager.clone();
        let cancelled = self.cancelled.clone();
        tokio::spawn(async move {
            while !cancelled.load(Ordering::Acquire) {
                sleep(Duration::from_millis(50)).await;
            }
            for id in task_ids.iter() {
                if let Err(error) = manager.cancel_queued(id) {
                    eprintln!("[tasks] cancellation for {id} failed: {error:#}");
                }
            }
            for worker_id in worker_ids.iter() {
                if let Err(error) = manager.cancel_owned_running(worker_id) {
                    eprintln!("[tasks] cancellation for worker {worker_id} failed: {error:#}");
                }
            }
        })
    }
}

enum WorkerEvent {
    Processed,
    Skipped(String),
}

async fn worker_loop<E>(
    mut worker: TaskWorker<E>,
    queue: Arc<Mutex<VecDeque<String>>>,
    cancelled: Arc<AtomicBool>,
    progress_tx: mpsc::UnboundedSender<WorkerEvent>,
) -> Result<()>
where
    E: TaskExecutor + 'static,
{
    loop {
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        let id = queue.lock().await.pop_front();
        let Some(id) = id else {
            break;
        };
        match worker.run_task(&id).await? {
            Some(_) => {
                let _ = progress_tx.send(WorkerEvent::Processed);
            }
            None => {
                let _ = progress_tx.send(WorkerEvent::Skipped(id));
            }
        }
    }
    Ok(())
}

fn progress_for(tasks: &[TaskRecord], skipped_ids: &HashSet<String>) -> TaskProgress {
    let mut progress = TaskProgress {
        total: tasks.len(),
        ..TaskProgress::default()
    };
    for task in tasks {
        if skipped_ids.contains(&task.id) {
            continue;
        }
        match task.status {
            TaskStatus::Queued => progress.queued += 1,
            TaskStatus::Running => progress.running += 1,
            TaskStatus::Completed => progress.completed += 1,
            TaskStatus::Failed => progress.failed += 1,
            TaskStatus::Cancelled => progress.cancelled += 1,
        }
    }
    progress
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_storage::DurableStorage;
    use crate::tasks::TaskIsolation;
    use anyhow::bail;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicUsize;
    use tempfile::TempDir;

    #[derive(Clone, Debug)]
    struct FakeExecutor {
        active: Arc<AtomicUsize>,
        maximum: Arc<AtomicUsize>,
        delay: Duration,
    }

    #[async_trait]
    impl TaskExecutor for FakeExecutor {
        async fn prepare(
            &mut self,
            _task: &TaskRecord,
            planned: &TaskIsolation,
        ) -> Result<TaskIsolation> {
            Ok(planned.clone())
        }

        async fn execute(
            &mut self,
            task: &TaskRecord,
            _isolation: &TaskIsolation,
        ) -> Result<String> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            sleep(self.delay).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            if task.prompt == "fail" {
                bail!("deliberate failure");
            }
            Ok(format!("diff for {}", task.id))
        }

        async fn cleanup(&mut self, _task: &TaskRecord, _isolation: &TaskIsolation) -> Result<()> {
            Ok(())
        }
    }

    fn fixture() -> (TempDir, TaskManager) {
        let temp = TempDir::new().unwrap();
        let storage = DurableStorage::new(temp.path().join("tasks.db")).unwrap();
        (temp, TaskManager::new(storage))
    }

    #[tokio::test]
    async fn coordinator_bounds_concurrency_and_aggregates_results() {
        let (_temp, manager) = fixture();
        for index in 0..8 {
            manager
                .create(&format!("task {index}"), "template")
                .unwrap();
        }
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let factory_active = active.clone();
        let factory_maximum = maximum.clone();
        let coordinator = TaskCoordinator::new(manager.clone(), 2, move || {
            Ok(FakeExecutor {
                active: factory_active.clone(),
                maximum: factory_maximum.clone(),
                delay: Duration::from_millis(15),
            })
        })
        .unwrap();

        let summary = coordinator.run_until_idle().await.unwrap();
        assert_eq!(summary.progress().total, 8);
        assert_eq!(summary.completed(), 8);
        assert_eq!(summary.failed(), 0);
        assert!(maximum.load(Ordering::SeqCst) <= 2);
        assert_eq!(manager.next_queued().unwrap(), None);
    }

    #[tokio::test]
    async fn coordinator_keeps_failed_tasks_in_aggregate() {
        let (_temp, manager) = fixture();
        manager.create("fail", "template").unwrap();
        manager.create("pass", "template").unwrap();
        let coordinator = TaskCoordinator::new(manager, 2, || {
            Ok(FakeExecutor {
                active: Arc::new(AtomicUsize::new(0)),
                maximum: Arc::new(AtomicUsize::new(0)),
                delay: Duration::from_millis(1),
            })
        })
        .unwrap();

        let summary = coordinator.run_until_idle().await.unwrap();
        assert_eq!(summary.completed(), 1);
        assert_eq!(summary.failed(), 1);
        assert_eq!(
            summary
                .tasks
                .iter()
                .filter(|task| task.status == TaskStatus::Failed)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn coordinator_cancels_queued_work_without_claiming_more() {
        let (_temp, manager) = fixture();
        for index in 0..4 {
            manager
                .create(&format!("task {index}"), "template")
                .unwrap();
        }
        let coordinator = Arc::new(
            TaskCoordinator::new(manager.clone(), 1, || {
                Ok(FakeExecutor {
                    active: Arc::new(AtomicUsize::new(0)),
                    maximum: Arc::new(AtomicUsize::new(0)),
                    delay: Duration::from_millis(100),
                })
            })
            .unwrap(),
        );
        let running = coordinator.clone();
        let handle = tokio::spawn(async move { running.run_until_idle().await.unwrap() });
        sleep(Duration::from_millis(10)).await;
        coordinator.cancel();
        let summary = handle.await.unwrap();
        assert!(summary.cancelled_by_user);
        assert_eq!(summary.cancelled(), 4);
        assert_eq!(summary.progress().finished(), 4);
    }

    #[tokio::test]
    async fn coordinator_does_not_claim_tasks_added_after_snapshot() {
        let (_temp, manager) = fixture();
        let first = manager.create("first", "template").unwrap();
        let coordinator = TaskCoordinator::new(manager.clone(), 1, || {
            Ok(FakeExecutor {
                active: Arc::new(AtomicUsize::new(0)),
                maximum: Arc::new(AtomicUsize::new(0)),
                delay: Duration::from_millis(300),
            })
        })
        .unwrap();

        let running = tokio::spawn(async move { coordinator.run_until_idle().await.unwrap() });
        for _ in 0..100 {
            if manager.get(&first.id).unwrap().unwrap().status == TaskStatus::Running {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            manager.get(&first.id).unwrap().unwrap().status,
            TaskStatus::Running
        );
        let late = manager
            .create("added after run started", "template")
            .unwrap();
        let summary = running.await.unwrap();

        assert_eq!(summary.progress().total, 1);
        assert_eq!(summary.completed(), 1);
        assert_eq!(summary.tasks[0].id, first.id);
        assert_eq!(
            manager.get(&late.id).unwrap().unwrap().status,
            TaskStatus::Queued
        );
    }

    #[tokio::test]
    async fn cancellation_does_not_cancel_another_coordinators_lease() {
        let (_temp, manager) = fixture();
        let task = manager.create("owned elsewhere", "template").unwrap();
        let other_worker = "other-coordinator-worker";
        manager
            .claim_with_isolation(
                &task.id,
                &TaskIsolation::planned(&task.id),
                other_worker,
                &(chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
            )
            .unwrap()
            .unwrap();

        let coordinator = TaskCoordinator::new(manager.clone(), 1, || {
            Ok(FakeExecutor {
                active: Arc::new(AtomicUsize::new(0)),
                maximum: Arc::new(AtomicUsize::new(0)),
                delay: Duration::from_millis(1),
            })
        })
        .unwrap();
        coordinator.cancel();
        let summary = coordinator.run_until_idle().await.unwrap();

        assert!(summary.cancelled_by_user);
        assert_eq!(summary.skipped(), 1);
        let progress = summary.progress();
        assert_eq!(
            progress.total,
            progress.queued
                + progress.running
                + progress.completed
                + progress.failed
                + progress.cancelled
                + progress.skipped
        );
        assert_eq!(
            manager.get(&task.id).unwrap().unwrap().status,
            TaskStatus::Running
        );
    }

    #[tokio::test]
    async fn coordinator_cancellation_preserves_other_coordinator_lease() {
        let (_temp, manager) = fixture();
        let task = manager.create("run elsewhere", "template").unwrap();
        let other = Arc::new(
            TaskCoordinator::new(manager.clone(), 1, || {
                Ok(FakeExecutor {
                    active: Arc::new(AtomicUsize::new(0)),
                    maximum: Arc::new(AtomicUsize::new(0)),
                    delay: Duration::from_millis(300),
                })
            })
            .unwrap(),
        );
        let other_run = other.clone();
        let other_handle = tokio::spawn(async move { other_run.run_until_idle().await.unwrap() });
        for _ in 0..100 {
            if manager.get(&task.id).unwrap().unwrap().status == TaskStatus::Running {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            manager.get(&task.id).unwrap().unwrap().status,
            TaskStatus::Running
        );

        let coordinator = TaskCoordinator::new(manager.clone(), 1, || {
            Ok(FakeExecutor {
                active: Arc::new(AtomicUsize::new(0)),
                maximum: Arc::new(AtomicUsize::new(0)),
                delay: Duration::from_millis(1),
            })
        })
        .unwrap();
        coordinator.cancel();
        let summary = coordinator.run_until_idle().await.unwrap();
        assert_eq!(summary.skipped(), 1);
        let progress = summary.progress();
        assert_eq!(
            progress.total,
            progress.queued
                + progress.running
                + progress.completed
                + progress.failed
                + progress.cancelled
                + progress.skipped
        );
        assert_eq!(
            manager.get(&task.id).unwrap().unwrap().status,
            TaskStatus::Running
        );

        // Stop the real second coordinator through the public cancellation
        // path so the test does not leave a live worker behind.
        manager.cancel(&task.id).unwrap();
        other_handle.await.unwrap();
    }
}
