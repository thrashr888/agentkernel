//! Daemon-integrated user job scheduling.
//!
//! This scheduler is intentionally separate from workspace lifecycle
//! scheduling.  It owns only config-defined jobs and delegates execution to
//! the same VmManager, orchestration store, and durable-object runtime used by
//! the HTTP API.

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::{Duration as TokioDuration, interval, timeout};

use crate::config::{Config, JobScheduleConfig, JobScheduleTarget};
use crate::orchestration_store::{CreateOrchestration, OrchestrationStore};
use crate::vmm::VmManager;

const DEFAULT_POLL_SECONDS: u64 = 15;
const NEXT_SCAN_MINUTES: i64 = 366 * 24 * 60;
const JOB_TIMEOUT: TokioDuration = TokioDuration::from_secs(300);

#[derive(Debug, Clone)]
struct JobDefinition {
    config: JobScheduleConfig,
    cron: crate::workspace_scheduler::CronSchedule,
    target: JobScheduleTarget,
}

/// Public status shape for `/schedules/configured`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct JobScheduleStatus {
    pub id: String,
    pub enabled: bool,
    pub cron: String,
    pub target: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
}

/// Result of a scheduler execution, including failures that were isolated
/// from the other jobs in the same daemon tick.
#[derive(Debug, Clone, Serialize)]
pub struct JobExecution {
    pub id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Handle retained by the daemon owner. Dropping it aborts the loop so a
/// server shutdown does not leave a detached scheduler task behind.
pub struct JobSchedulerHandle(JoinHandle<()>);

impl Drop for JobSchedulerHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Config-driven scheduler and execution facade used by both the daemon loop
/// and the `/schedules` API.
pub struct JobScheduler {
    jobs: BTreeMap<String, JobDefinition>,
    store: Arc<OrchestrationStore>,
    vm_manager: Arc<OnceLock<Arc<RwLock<VmManager>>>>,
    poll_seconds: u64,
}

impl JobScheduler {
    /// Validate and compile all configured schedules. Errors include the
    /// stable schedule id so startup failures are actionable.
    pub fn from_config(
        config: &Config,
        store: Arc<OrchestrationStore>,
        vm_manager: Arc<OnceLock<Arc<RwLock<VmManager>>>>,
    ) -> Result<Option<Self>> {
        if config.schedules.is_empty() {
            return Ok(None);
        }

        let mut jobs = BTreeMap::new();
        for schedule in &config.schedules {
            let id = schedule.id.trim();
            if id.is_empty() {
                bail!("schedule id must not be empty");
            }
            if jobs.contains_key(id) {
                bail!("schedule '{id}': duplicate schedule id");
            }
            let cron = crate::workspace_scheduler::CronSchedule::parse(&schedule.cron)
                .with_context(|| format!("schedule '{id}': invalid cron expression"))?;
            let target = schedule
                .resolve_target()
                .with_context(|| format!("schedule '{id}': invalid target"))?;
            validate_target(&target).with_context(|| format!("schedule '{id}': invalid target"))?;
            jobs.insert(
                id.to_string(),
                JobDefinition {
                    config: schedule.clone(),
                    cron,
                    target,
                },
            );
        }

        Ok(Some(Self {
            jobs,
            store,
            vm_manager,
            poll_seconds: DEFAULT_POLL_SECONDS,
        }))
    }

    /// Start the periodic UTC cron loop.
    pub fn spawn(self: Arc<Self>) -> JobSchedulerHandle {
        let task = tokio::spawn(async move {
            let mut ticker = interval(TokioDuration::from_secs(self.poll_seconds));
            loop {
                ticker.tick().await;
                let now = Utc::now();
                if let Err(error) = self.run_due(now).await {
                    // A store-wide failure is reported, but individual target
                    // failures are recorded and never escape this loop.
                    eprintln!("[scheduler] tick failed: {error:#}");
                }
            }
        });
        JobSchedulerHandle(task)
    }

    /// Return status for all configured jobs, deriving next-run time in UTC.
    pub fn list_status(&self, now: DateTime<Utc>) -> Result<Vec<JobScheduleStatus>> {
        self.jobs
            .values()
            .map(|job| self.status_for(job, now))
            .collect()
    }

    /// Return one configured job's status.
    pub fn get_status(&self, id: &str, now: DateTime<Utc>) -> Result<Option<JobScheduleStatus>> {
        self.jobs
            .get(id)
            .map(|job| self.status_for(job, now))
            .transpose()
    }

    /// Trigger a configured job immediately. Manual triggers intentionally do
    /// not depend on cron matching, but still persist the truthful outcome.
    pub async fn trigger(&self, id: &str) -> Result<JobExecution> {
        let job = self
            .jobs
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("schedule '{id}' not found"))?
            .clone();
        let now = Utc::now();
        let minute = now.timestamp().div_euclid(60);
        let next = next_run(&job.cron, now).map(|at| at.to_rfc3339());
        self.store.start_scheduled_job(
            &job.config.id,
            minute,
            &now.to_rfc3339(),
            next.as_deref(),
        )?;
        let execution = self.execute_with_timeout(&job).await;
        self.persist_result(&job, now, minute, &execution, next.as_deref())?;
        Ok(execution)
    }

    /// Run all jobs matching the current UTC minute. The durable atomic claim
    /// makes repeated loop ticks and multiple daemon instances idempotent for
    /// a given schedule/minute.
    pub async fn run_due(&self, now: DateTime<Utc>) -> Result<Vec<JobExecution>> {
        let minute = now.timestamp().div_euclid(60);
        let minute_at = minute_start(minute)?;
        let mut claims = Vec::new();
        for job in self.jobs.values() {
            if !job.config.enabled || !job.cron.matches(minute_at) {
                continue;
            }
            let next = next_run(&job.cron, minute_at).map(|at| at.to_rfc3339());
            if !self.store.claim_scheduled_job_minute(
                &job.config.id,
                minute,
                &now.to_rfc3339(),
                next.as_deref(),
            )? {
                continue;
            }
            claims.push((job.clone(), next));
        }

        // Do not let one slow sandbox command hold up unrelated jobs. Each
        // action has a hard upper bound and all claimed jobs are awaited before
        // returning so their outcomes are persisted deterministically.
        let mut pending = FuturesUnordered::new();
        for (job, next) in claims {
            let next = next.clone();
            pending.push(async move {
                let execution = self.execute_with_timeout(&job).await;
                self.persist_result(&job, now, minute, &execution, next.as_deref())
                    .map(|()| execution)
            });
        }

        let mut results = Vec::new();
        let mut first_persist_error = None;
        while let Some(result) = pending.next().await {
            match result {
                Ok(execution) => results.push(execution),
                Err(error) => {
                    first_persist_error.get_or_insert(error);
                }
            }
        }
        if let Some(error) = first_persist_error {
            return Err(error);
        }
        Ok(results)
    }

    fn status_for(&self, job: &JobDefinition, now: DateTime<Utc>) -> Result<JobScheduleStatus> {
        let state = self.store.get_scheduled_job_run(&job.config.id)?;
        let next_run_at = if job.config.enabled {
            state
                .as_ref()
                .and_then(|value| value.next_run_at.clone())
                .or_else(|| next_run(&job.cron, now).map(|at| at.to_rfc3339()))
        } else {
            None
        };
        let status = if !job.config.enabled {
            "disabled".to_string()
        } else {
            state
                .as_ref()
                .and_then(|value| value.last_status.clone())
                .unwrap_or_else(|| "idle".to_string())
        };
        Ok(JobScheduleStatus {
            id: job.config.id.clone(),
            enabled: job.config.enabled,
            cron: job.config.cron.clone(),
            target: target_kind(&job.target).to_string(),
            status,
            last_run_at: state.as_ref().and_then(|value| value.last_run_at.clone()),
            last_error: state.as_ref().and_then(|value| value.last_error.clone()),
            next_run_at,
        })
    }

    async fn execute(&self, job: &JobDefinition) -> JobExecution {
        let result = match &job.target {
            JobScheduleTarget::SandboxCommand { sandbox, command } => {
                self.execute_sandbox(sandbox, command).await
            }
            JobScheduleTarget::Orchestration { name, input } => self
                .store
                .create(CreateOrchestration {
                    name: name.clone(),
                    input: input.clone(),
                })
                .map(|record| serde_json::to_string(&record).unwrap_or_default())
                .context("failed to start orchestration"),
            JobScheduleTarget::ObjectMethod {
                class,
                object_id,
                method,
                args,
            } => self.execute_object(class, object_id, method, args).await,
        };
        match result {
            Ok(output) => JobExecution {
                id: job.config.id.clone(),
                status: "success".to_string(),
                output: Some(output),
                error: None,
            },
            Err(error) => JobExecution {
                id: job.config.id.clone(),
                status: "failed".to_string(),
                output: None,
                error: Some(error.to_string()),
            },
        }
    }

    async fn execute_with_timeout(&self, job: &JobDefinition) -> JobExecution {
        match timeout(JOB_TIMEOUT, self.execute(job)).await {
            Ok(execution) => execution,
            Err(_) => JobExecution {
                id: job.config.id.clone(),
                status: "failed".to_string(),
                output: None,
                error: Some(format!("job exceeded {} seconds", JOB_TIMEOUT.as_secs())),
            },
        }
    }

    async fn execute_sandbox(&self, sandbox: &str, command: &[String]) -> Result<String> {
        let manager = self.ensure_manager()?;
        let mut manager = manager.write().await;
        manager
            .exec_cmd(sandbox, command)
            .await
            .with_context(|| format!("sandbox command failed in '{sandbox}'"))
    }

    async fn execute_object(
        &self,
        class: &str,
        object_id: &str,
        method: &str,
        args: &Option<serde_json::Value>,
    ) -> Result<String> {
        let manager = self.ensure_manager()?;
        let body = serde_json::to_vec(args.as_ref().unwrap_or(&serde_json::Value::Null))?;
        let (status, output) = crate::object_runtime::handle_object_call(
            &self.store,
            &manager,
            class,
            object_id,
            method,
            Bytes::from(body),
        )
        .await?;
        if !(200..400).contains(&status) {
            bail!("durable object method returned HTTP {status}: {output}");
        }
        Ok(output)
    }

    fn ensure_manager(&self) -> Result<Arc<RwLock<VmManager>>> {
        if let Some(manager) = self.vm_manager.get() {
            return Ok(manager.clone());
        }
        let manager = Arc::new(RwLock::new(VmManager::new()?));
        let _ = self.vm_manager.set(manager);
        self.vm_manager
            .get()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("VmManager could not be initialized"))
    }

    fn persist_result(
        &self,
        job: &JobDefinition,
        now: DateTime<Utc>,
        minute: i64,
        execution: &JobExecution,
        next_run_at: Option<&str>,
    ) -> Result<()> {
        crate::audit::log_event(crate::audit::AuditEvent::ScheduleTriggered {
            schedule_id: job.config.id.clone(),
            schedule_name: job.config.id.clone(),
            method: target_kind(&job.target).to_string(),
        });
        self.store.finish_scheduled_job(
            &job.config.id,
            minute,
            &now.to_rfc3339(),
            execution.status == "success",
            execution.error.as_deref(),
            next_run_at,
        )
    }
}

fn validate_target(target: &JobScheduleTarget) -> Result<()> {
    match target {
        JobScheduleTarget::SandboxCommand { sandbox, command } => {
            crate::validation::validate_sandbox_name(sandbox)?;
            if command.is_empty() || command.iter().any(|part| part.trim().is_empty()) {
                bail!("command must contain at least one non-empty argument");
            }
        }
        JobScheduleTarget::Orchestration { name, .. } => {
            if name.trim().is_empty() {
                bail!("orchestration name must not be empty");
            }
        }
        JobScheduleTarget::ObjectMethod {
            class,
            object_id,
            method,
            ..
        } => {
            if class.trim().is_empty() || object_id.trim().is_empty() || method.trim().is_empty() {
                bail!("object class, object_id, and method are required");
            }
        }
    }
    Ok(())
}

fn target_kind(target: &JobScheduleTarget) -> &'static str {
    match target {
        JobScheduleTarget::SandboxCommand { .. } => "sandbox_command",
        JobScheduleTarget::Orchestration { .. } => "orchestration",
        JobScheduleTarget::ObjectMethod { .. } => "object_method",
    }
}

fn minute_start(minute: i64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp(minute.saturating_mul(60), 0)
        .ok_or_else(|| anyhow::anyhow!("cron minute is outside the supported UTC range"))
}

fn next_run(
    cron: &crate::workspace_scheduler::CronSchedule,
    after: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let minute = after.timestamp().div_euclid(60);
    (1..=NEXT_SCAN_MINUTES).find_map(|offset| {
        let candidate = minute_start(minute + offset).ok()?;
        cron.matches(candidate).then_some(candidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::durable_storage::DurableStorage;
    use tempfile::tempdir;

    fn config_with(schedule: &str) -> Config {
        Config::from_str(&format!(
            r#"
[sandbox]
name = "test"
[[schedule]]
{schedule}
"#
        ))
        .unwrap()
    }

    fn scheduler(config: &Config) -> JobScheduler {
        let dir = tempdir().unwrap();
        // Keep the temporary directory alive for this test process by leaking
        // it; the store itself owns the SQLite connection path.
        let path = dir.keep().join("scheduler.db");
        let store = Arc::new(OrchestrationStore::new(DurableStorage::new(path).unwrap()));
        JobScheduler::from_config(config, store, Arc::new(OnceLock::new()))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn config_requires_one_target() {
        let config = config_with("id = \"broken\"\ncron = \"* * * * *\"");
        let error = JobScheduler::from_config(
            &config,
            Arc::new(OrchestrationStore::new(
                DurableStorage::new(tempdir().unwrap().keep().join("db")).unwrap(),
            )),
            Arc::new(OnceLock::new()),
        )
        .err()
        .expect("invalid schedule should fail");
        let error = format!("{error:#}");
        assert!(error.contains("schedule 'broken'"));
        assert!(error.contains("exactly one target"));
    }

    #[test]
    fn parser_supports_flat_and_tagged_targets() {
        let flat = config_with(
            "id = \"flat\"\ncron = \"*/5 * * * *\"\ntype = \"sandbox_command\"\nsandbox = \"worker\"\ncommand = [\"echo\", \"ok\"]",
        );
        assert!(matches!(
            scheduler(&flat).jobs["flat"].target,
            JobScheduleTarget::SandboxCommand { .. }
        ));

        let tagged = config_with(
            "id = \"tagged\"\ncron = \"0 * * * *\"\ntarget = { type = \"orchestration\", name = \"nightly\" }",
        );
        assert!(matches!(
            scheduler(&tagged).jobs["tagged"].target,
            JobScheduleTarget::Orchestration { .. }
        ));
    }

    #[test]
    fn status_derives_next_run_in_utc() {
        let config =
            config_with("id = \"hourly\"\ncron = \"0 * * * *\"\norchestration = \"hourly\"");
        let scheduler = scheduler(&config);
        let now = DateTime::parse_from_rfc3339("2026-08-23T12:34:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let status = scheduler.get_status("hourly", now).unwrap().unwrap();
        assert_eq!(
            status.next_run_at.as_deref(),
            Some("2026-08-23T13:00:00+00:00")
        );
    }

    #[tokio::test]
    async fn claim_prevents_duplicate_loop_ticks() {
        let config =
            config_with("id = \"hourly\"\ncron = \"0 * * * *\"\norchestration = \"hourly\"");
        let scheduler = scheduler(&config);
        let now = DateTime::parse_from_rfc3339("2026-08-23T12:00:03Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(scheduler.run_due(now).await.unwrap().len(), 1);
        assert!(
            scheduler
                .run_due(now + chrono::Duration::seconds(20))
                .await
                .unwrap()
                .is_empty()
        );
    }
}
