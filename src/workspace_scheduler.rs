//! Config-driven workspace lifecycle scheduling.
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Timelike, Utc};
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};

use crate::config::WorkspaceSchedulingConfig;
use crate::vmm::VmManager;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceScheduleReport {
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub marked_dormant: Vec<String>,
    pub removed: Vec<String>,
    pub archived: Vec<String>,
    pub errors: Vec<String>,
}

impl WorkspaceScheduleReport {
    fn merge_lifecycle(&mut self, result: crate::vmm::LifecycleReconcileResult) {
        self.stopped.extend(result.stopped);
        self.archived.extend(result.archived);
        self.removed.extend(result.removed);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    minute: CronField,
    hour: CronField,
    day_of_month: CronField,
    month: CronField,
    day_of_week: CronField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CronField {
    values: BTreeSet<u32>,
    wildcard: bool,
}

impl CronField {
    fn parse(value: &str, min: u32, max: u32, day_of_week: bool) -> Result<Self> {
        let value = value.trim();
        if value.is_empty() {
            bail!("cron field is empty");
        }
        let wildcard = value == "*" || value.starts_with("*/");
        let mut values = BTreeSet::new();
        for component in value.split(',') {
            if component.trim().is_empty() {
                bail!("cron field contains an empty list item");
            }
            let (base, step) = match component.split_once('/') {
                Some((base, step)) => {
                    let step = step
                        .parse::<u32>()
                        .with_context(|| format!("invalid cron step '{step}'"))?;
                    if step == 0 {
                        bail!("cron step must be greater than zero");
                    }
                    (base, step)
                }
                None => (component, 1),
            };
            let (start, end) = if base == "*" {
                (min, max)
            } else if let Some((start, end)) = base.split_once('-') {
                let start = parse_cron_number(start, min, max, day_of_week)?;
                let end = parse_cron_number(end, min, max, day_of_week)?;
                if start > end {
                    bail!("cron range '{base}' is descending");
                }
                (start, end)
            } else {
                let single = parse_cron_number(base, min, max, day_of_week)?;
                (single, if component.contains('/') { max } else { single })
            };
            let mut current = start;
            while current <= end {
                values.insert(if day_of_week && current == 7 {
                    0
                } else {
                    current
                });
                match current.checked_add(step) {
                    Some(next) if next > current => current = next,
                    _ => break,
                }
            }
        }
        if values.is_empty() {
            bail!("cron field '{value}' produced no values");
        }
        Ok(Self { values, wildcard })
    }

    fn contains(&self, value: u32) -> bool {
        self.values.contains(&value)
    }
}

fn parse_cron_number(value: &str, min: u32, max: u32, day_of_week: bool) -> Result<u32> {
    let parsed = value
        .parse::<u32>()
        .with_context(|| format!("invalid cron value '{value}'"))?;
    let upper = if day_of_week { 7 } else { max };
    if parsed < min || parsed > upper {
        bail!("cron value {parsed} is outside {min}-{upper}");
    }
    Ok(parsed)
}

impl CronSchedule {
    pub fn parse(expression: &str) -> Result<Self> {
        let expression = match expression.trim() {
            "@hourly" => "0 * * * *",
            "@daily" | "@midnight" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            "@monthly" => "0 0 1 * *",
            "@yearly" | "@annually" => "0 0 1 1 *",
            value => value,
        };
        let fields: Vec<_> = expression.split_whitespace().collect();
        if fields.len() != 5 {
            bail!("cron must have five fields (minute hour day-of-month month day-of-week)");
        }
        Ok(Self {
            minute: CronField::parse(fields[0], 0, 59, false)?,
            hour: CronField::parse(fields[1], 0, 23, false)?,
            day_of_month: CronField::parse(fields[2], 1, 31, false)?,
            month: CronField::parse(fields[3], 1, 12, false)?,
            day_of_week: CronField::parse(fields[4], 0, 6, true)?,
        })
    }

    pub fn matches(&self, at: DateTime<Utc>) -> bool {
        if !self.minute.contains(at.minute())
            || !self.hour.contains(at.hour())
            || !self.month.contains(at.month())
        {
            return false;
        }
        let dom = self.day_of_month.contains(at.day());
        let dow = self
            .day_of_week
            .contains(at.weekday().num_days_from_sunday());
        if self.day_of_month.wildcard || self.day_of_week.wildcard {
            dom && dow
        } else {
            dom || dow
        }
    }
}

pub struct WorkspaceScheduler {
    config: WorkspaceSchedulingConfig,
    cron: Option<CronSchedule>,
    last_autostart_minute: Option<i64>,
}

impl WorkspaceScheduler {
    pub fn new(config: WorkspaceSchedulingConfig) -> Result<Self> {
        let cron = config
            .autostart_cron
            .as_deref()
            .map(CronSchedule::parse)
            .transpose()
            .context("invalid workspace autostart cron")?;
        Ok(Self {
            config,
            cron,
            last_autostart_minute: None,
        })
    }

    pub async fn reconcile(
        &mut self,
        manager: &mut VmManager,
        now: DateTime<Utc>,
    ) -> Result<WorkspaceScheduleReport> {
        let mut report = WorkspaceScheduleReport::default();
        report.merge_lifecycle(manager.reconcile_lifecycle(false).await?);
        let names = manager.sandbox_names();

        let minute = now.timestamp().div_euclid(60);
        let autostart_due = self
            .cron
            .as_ref()
            .is_some_and(|cron| cron.matches(now) && self.last_autostart_minute != Some(minute));
        if autostart_due {
            self.last_autostart_minute = Some(minute);
            for name in &names {
                let Some(state) = manager.get_state(name) else {
                    continue;
                };
                if state.archived_at.is_some()
                    || state.dormant_at.is_some()
                    || manager.is_running(name)
                {
                    continue;
                }
                match manager.start(name).await {
                    Ok(()) => report.started.push(name.clone()),
                    Err(error) => report.errors.push(format!("start {name}: {error}")),
                }
            }
        }

        if let Some(stop_age) = self.config.autostop_after_minutes.map(minutes_to_seconds) {
            for name in &names {
                let Some(state) = manager.get_state(name) else {
                    continue;
                };
                if state.archived_at.is_some() || state.dormant_at.is_some() {
                    continue;
                }
                let idle = manager
                    .activity_time(name)
                    .map(|at| now.signed_duration_since(at).num_seconds().max(0) as u64)
                    .unwrap_or(0);
                if idle < stop_age || !manager.is_running(name) {
                    continue;
                }
                match manager.stop(name).await {
                    Ok(()) => report.stopped.push(name.clone()),
                    Err(error) => report.errors.push(format!("stop {name}: {error}")),
                }
            }
        }

        if let Some(dormant_age) = self.config.dormant_after_days.map(days_to_seconds) {
            for name in &names {
                let Some(state) = manager.get_state(name) else {
                    continue;
                };
                if state.archived_at.is_some()
                    || state.dormant_at.is_some()
                    || manager.is_running(name)
                {
                    continue;
                }
                let idle = manager
                    .activity_time(name)
                    .map(|at| now.signed_duration_since(at).num_seconds().max(0) as u64)
                    .unwrap_or(0);
                if idle < dormant_age {
                    continue;
                }
                let reason = format!("unused for {idle}s (threshold={dormant_age}s)");
                match manager.mark_dormant(name, &now.to_rfc3339(), &reason) {
                    Ok(()) => report.marked_dormant.push(name.clone()),
                    Err(error) => report.errors.push(format!("mark dormant {name}: {error}")),
                }
            }
        }

        if let Some(remove_age) = self.config.remove_dormant_after_days.map(days_to_seconds) {
            for name in &names {
                if manager.is_running(name) {
                    continue;
                }
                let Some(dormant_at) = manager.dormant_time(name) else {
                    continue;
                };
                let dormant_for = elapsed_seconds(dormant_at, now);
                if dormant_for < remove_age {
                    continue;
                }
                match manager.remove(name).await {
                    Ok(()) => report.removed.push(name.clone()),
                    Err(error) => report
                        .errors
                        .push(format!("remove dormant {name}: {error}")),
                }
            }
        }
        Ok(report)
    }
}

fn minutes_to_seconds(minutes: u64) -> u64 {
    minutes.saturating_mul(60)
}
fn days_to_seconds(days: u64) -> u64 {
    days.saturating_mul(24 * 60 * 60)
}
fn elapsed_seconds(since: DateTime<Utc>, now: DateTime<Utc>) -> u64 {
    now.signed_duration_since(since).num_seconds().max(0) as u64
}

/// Start the scheduler daemon for an active config.
pub fn spawn_enforcement_loop(
    manager: Arc<RwLock<VmManager>>,
    config: WorkspaceSchedulingConfig,
) -> Option<JoinHandle<()>> {
    if !config.is_active() {
        return None;
    }
    let interval_seconds = config.check_interval_seconds.max(1);
    let mut scheduler = match WorkspaceScheduler::new(config) {
        Ok(scheduler) => scheduler,
        Err(error) => {
            eprintln!("[workspace-scheduler] disabled: {error:#}");
            return None;
        }
    };
    eprintln!("[workspace-scheduler] enforcement enabled (poll={interval_seconds}s)");
    Some(tokio::spawn(async move {
        loop {
            let result = {
                let mut manager = manager.write().await;
                scheduler.reconcile(&mut manager, Utc::now()).await
            };
            match result {
                Ok(report) if !report.errors.is_empty() => {
                    for error in report.errors {
                        eprintln!("[workspace-scheduler] {error}");
                    }
                }
                Err(error) => eprintln!("[workspace-scheduler] reconciliation failed: {error:#}"),
                _ => {}
            }
            sleep(Duration::from_secs(interval_seconds)).await;
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }

    #[test]
    fn cron_supports_ranges_steps_and_weekdays() {
        let cron = CronSchedule::parse("*/15 9-10 * * 1-5").unwrap();
        assert!(cron.matches(at("2026-08-24T09:15:00Z")));
        assert!(!cron.matches(at("2026-08-24T09:16:00Z")));
        assert!(!cron.matches(at("2026-08-23T09:15:00Z")));
    }

    #[test]
    fn cron_uses_standard_or_semantics_for_day_fields() {
        let cron = CronSchedule::parse("0 0 1 * 1").unwrap();
        assert!(cron.matches(at("2026-06-01T00:00:00Z")));
        assert!(cron.matches(at("2026-06-08T00:00:00Z")));
        assert!(!cron.matches(at("2026-06-02T00:00:00Z")));
    }

    #[test]
    fn cron_rejects_invalid_expressions() {
        assert!(CronSchedule::parse("0 9 * *").is_err());
        assert!(CronSchedule::parse("61 * * * *").is_err());
        assert!(CronSchedule::parse("*/0 * * * *").is_err());
    }

    #[test]
    fn config_aliases_parse() {
        let config: WorkspaceSchedulingConfig = toml::from_str(
            r#"
enabled = true
autostop_minutes = 30
autostart_schedule = "0 9 * * 1-5"
dormant_days = 7
delete_dormant_after_days = 30
interval = 5
"#,
        )
        .unwrap();
        assert_eq!(config.autostop_after_minutes, Some(30));
        assert_eq!(config.autostart_cron.as_deref(), Some("0 9 * * 1-5"));
        assert_eq!(config.dormant_after_days, Some(7));
        assert_eq!(config.remove_dormant_after_days, Some(30));
        assert_eq!(config.check_interval_seconds, 5);
    }

    #[test]
    fn dormant_retention_uses_time_since_dormant_mark() {
        let now = at("2026-08-24T00:00:00Z");
        assert_eq!(
            elapsed_seconds(at("2026-08-23T00:00:00Z"), now),
            days_to_seconds(1)
        );
        assert_eq!(elapsed_seconds(at("2026-08-25T00:00:00Z"), now), 0);
    }
}
