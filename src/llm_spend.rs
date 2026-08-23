//! Durable, identity-aware aggregation for intercepted LLM traffic.
//!
//! The interceptor deliberately records only bounded dimensions and token
//! counters. Prompt/response bodies, headers, API keys, and provider URLs
//! are never persisted here. Aggregation is kept in SQLite at daily
//! granularity so the amount of data is bounded by the number of dimensions
//! and the retention window rather than by request volume.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Maximum number of days retained by the aggregate database.
pub const RETENTION_DAYS: i64 = 180;
/// Maximum number of rows returned by one API request.
pub const MAX_PAGE_SIZE: usize = 200;
/// Maximum number of aggregate rows retained for one tenant/user and UTC day.
///
/// The final row is reserved for the overflow bucket. This bounds storage
/// even when a trusted user emits unbounded distinct project/model labels,
/// while preserving user-level access-control filtering.
pub const MAX_DAILY_ROWS_PER_TENANT_USER: i64 = 10_000;
/// Maximum offset accepted by the API and durable query layer.
pub const MAX_QUERY_OFFSET: usize = 100_000;
const MAX_DIMENSION_LENGTH: usize = 128;
const OVERFLOW_DIMENSION: &str = "__overflow__";

/// A query over the durable LLM usage aggregates.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LlmSpendFilter {
    pub tenant: Option<String>,
    pub agent: Option<String>,
    pub user: Option<String>,
    pub project: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// One daily aggregate row. The bucket is an ISO date in UTC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmSpendMetric {
    pub bucket: String,
    pub tenant: String,
    pub agent: String,
    pub user: String,
    pub project: String,
    pub provider: String,
    pub model: String,
    pub request_count: u64,
    pub streaming_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub last_request: String,
}

/// Explicitly states whether a monetary estimate is available.
///
/// AgentKernel does not ship provider pricing, so this remains unavailable
/// until a caller supplies a separately versioned pricing source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonetaryCostStatus {
    pub available: bool,
    pub currency: Option<String>,
    pub reason: String,
}

impl Default for MonetaryCostStatus {
    fn default() -> Self {
        Self {
            available: false,
            currency: None,
            reason: "Provider pricing is not configured; values are token usage only".into(),
        }
    }
}

/// Paginated response for the authenticated spend API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSpendReport {
    pub metrics: Vec<LlmSpendMetric>,
    pub next_offset: Option<usize>,
    pub retention_days: i64,
    pub monetary_cost: MonetaryCostStatus,
}

/// Small metadata object used to annotate events before aggregation.
#[derive(Debug, Clone, Default)]
pub struct LlmIdentity {
    pub tenant: Option<String>,
    pub agent: Option<String>,
    pub user: Option<String>,
    pub project: Option<String>,
}

/// SQLite-backed aggregate store.
#[derive(Debug, Clone)]
pub struct LlmSpendStore {
    path: PathBuf,
}

impl LlmSpendStore {
    /// Open or create a store at path.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let store = Self { path: path.into() };
        store.bootstrap()?;
        Ok(store)
    }

    /// Open the default per-user AgentKernel aggregate database.
    pub fn open_default() -> Result<Self> {
        let path = std::env::var_os("AGENTKERNEL_LLM_USAGE_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local/share/agentkernel/llm-usage.db")
            });
        Self::new(path)
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connection(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path).with_context(|| {
            format!("failed to open LLM usage database {}", self.path.display())
        })?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        Ok(conn)
    }

    fn bootstrap(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create LLM usage directory {}", parent.display())
            })?;
        }
        let conn = self.connection()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS llm_usage_daily (
                bucket TEXT NOT NULL,
                tenant TEXT NOT NULL,
                agent TEXT NOT NULL,
                user_id TEXT NOT NULL,
                project TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                request_count INTEGER NOT NULL DEFAULT 0,
                streaming_count INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                last_request TEXT NOT NULL,
                PRIMARY KEY(bucket, tenant, agent, user_id, project, provider, model)
            );
            CREATE INDEX IF NOT EXISTS idx_llm_usage_daily_bucket
                ON llm_usage_daily(bucket DESC);
            CREATE INDEX IF NOT EXISTS idx_llm_usage_daily_user
                ON llm_usage_daily(tenant, user_id, bucket DESC);
            CREATE INDEX IF NOT EXISTS idx_llm_usage_daily_project
                ON llm_usage_daily(tenant, project, bucket DESC);",
        )?;
        Ok(())
    }

    /// Record one intercepted event. Only dimensions and counters are kept.
    pub fn record_event(&self, event: &crate::llm_intercept::LlmEvent) -> Result<()> {
        let bucket = event_bucket(&event.timestamp)?;
        let identity = LlmIdentity {
            tenant: event.tenant.clone(),
            agent: event.agent.clone(),
            user: event.user.clone(),
            project: event.project.clone(),
        };
        let tenant = bounded_dimension(identity.tenant.as_deref().unwrap_or("local"));
        let agent = bounded_dimension(identity.agent.as_deref().unwrap_or("unknown"));
        let user = bounded_dimension(identity.user.as_deref().unwrap_or("unknown"));
        let project = bounded_dimension(identity.project.as_deref().unwrap_or("unknown"));
        let provider = bounded_dimension(&event.provider);
        let model = bounded_dimension(event.model.as_deref().unwrap_or("unknown"));
        let request_count: i64 = 1;
        let streaming_count: i64 = i64::from(event.streaming);
        let input_tokens = i64::try_from(event.input_tokens.unwrap_or(0)).unwrap_or(i64::MAX);
        let output_tokens = i64::try_from(event.output_tokens.unwrap_or(0)).unwrap_or(i64::MAX);
        let total_tokens = i64::try_from(event.total_tokens.unwrap_or_else(|| {
            event
                .input_tokens
                .unwrap_or(0)
                .saturating_add(event.output_tokens.unwrap_or(0))
        }))
        .unwrap_or(i64::MAX);

        let mut conn = self.connection()?;
        let tx = conn.transaction()?;
        let normal_key_exists: bool = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM llm_usage_daily
                WHERE bucket = ?1 AND tenant = ?2 AND agent = ?3 AND user_id = ?4
                  AND project = ?5 AND provider = ?6 AND model = ?7
            )",
            params![bucket, tenant, agent, user, project, provider, model],
            |row| row.get(0),
        )?;
        let (agent, user, project, provider, model) = if normal_key_exists {
            (agent, user, project, provider, model)
        } else {
            let row_count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM llm_usage_daily
                 WHERE bucket = ?1 AND tenant = ?2 AND user_id = ?3",
                params![bucket, tenant, user],
                |row| row.get(0),
            )?;
            if row_count >= MAX_DAILY_ROWS_PER_TENANT_USER - 1 {
                (
                    OVERFLOW_DIMENSION.to_string(),
                    user,
                    OVERFLOW_DIMENSION.to_string(),
                    OVERFLOW_DIMENSION.to_string(),
                    OVERFLOW_DIMENSION.to_string(),
                )
            } else {
                (agent, user, project, provider, model)
            }
        };
        tx.execute(
            "INSERT INTO llm_usage_daily
                (bucket, tenant, agent, user_id, project, provider, model,
                 request_count, streaming_count, input_tokens, output_tokens,
                 total_tokens, last_request)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(bucket, tenant, agent, user_id, project, provider, model)
             DO UPDATE SET
                 request_count = request_count + excluded.request_count,
                 streaming_count = streaming_count + excluded.streaming_count,
                 input_tokens = input_tokens + excluded.input_tokens,
                 output_tokens = output_tokens + excluded.output_tokens,
                 total_tokens = total_tokens + excluded.total_tokens,
                 last_request = CASE WHEN excluded.last_request > last_request
                                     THEN excluded.last_request ELSE last_request END",
            params![
                bucket,
                tenant,
                agent,
                user,
                project,
                provider,
                model,
                request_count,
                streaming_count,
                input_tokens,
                output_tokens,
                total_tokens,
                event.timestamp,
            ],
        )?;

        let cutoff = (Utc::now() - Duration::days(RETENTION_DAYS)).date_naive();
        tx.execute(
            "DELETE FROM llm_usage_daily WHERE bucket < ?1",
            [cutoff.format("%Y-%m-%d").to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Read JSONL LLM events produced by an existing proxy file hook.
    ///
    /// Legacy lines without identity fields remain visible under unknown,
    /// which preserves compatibility without guessing ownership.
    #[allow(dead_code)]
    pub fn ingest_jsonl(&self, path: impl AsRef<Path>) -> Result<usize> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("failed to read LLM event log {}", path.as_ref().display()))?;
        let mut ingested = 0;
        for (line_number, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<crate::llm_intercept::LlmEvent>(line) {
                Ok(event) => {
                    self.record_event(&event).with_context(|| {
                        format!("failed to aggregate LLM event at line {}", line_number + 1)
                    })?;
                    ingested += 1;
                }
                Err(error) => {
                    eprintln!(
                        "[llm-spend] skipping malformed event at {}:{}: {}",
                        path.as_ref().display(),
                        line_number + 1,
                        error
                    );
                }
            }
        }
        Ok(ingested)
    }

    pub fn query(&self, filter: &LlmSpendFilter) -> Result<LlmSpendReport> {
        let limit = match filter.limit {
            None => 100,
            Some(value) if (1..=MAX_PAGE_SIZE).contains(&value) => value,
            Some(value) => bail!("limit must be between 1 and {MAX_PAGE_SIZE}, got {value}"),
        };
        let offset = match filter.offset {
            None => 0,
            Some(value) if value <= MAX_QUERY_OFFSET => value,
            Some(value) => bail!("offset must be at most {MAX_QUERY_OFFSET}, got {value}"),
        };
        let from = parse_bound(filter.from.as_deref(), "from")?;
        let to = parse_bound(filter.to.as_deref(), "to")?;
        if let (Some(from), Some(to)) = (&from, &to)
            && from > to
        {
            bail!("from must be earlier than or equal to to");
        }

        let conn = self.connection()?;
        let mut sql = String::from(
            "SELECT bucket, tenant, agent, user_id, project, provider, model,
                    request_count, streaming_count, input_tokens,
                    output_tokens, total_tokens, last_request
             FROM llm_usage_daily WHERE 1=1",
        );
        let mut values: Vec<String> = Vec::new();
        if let Some(tenant) = filter.tenant.as_deref() {
            sql.push_str(" AND tenant = ?");
            values.push(bounded_dimension(tenant));
        }
        if let Some(agent) = filter.agent.as_deref() {
            sql.push_str(" AND agent = ?");
            values.push(bounded_dimension(agent));
        }
        if let Some(user) = filter.user.as_deref() {
            sql.push_str(" AND user_id = ?");
            values.push(bounded_dimension(user));
        }
        if let Some(project) = filter.project.as_deref() {
            sql.push_str(" AND project = ?");
            values.push(bounded_dimension(project));
        }
        if let Some(from) = from.as_deref() {
            sql.push_str(" AND bucket >= ?");
            values.push(from.to_string());
        }
        if let Some(to) = to.as_deref() {
            sql.push_str(" AND bucket <= ?");
            values.push(to.to_string());
        }
        sql.push_str(" ORDER BY bucket DESC, last_request DESC, agent, user_id, project, provider, model LIMIT ? OFFSET ?");

        let mut stmt = conn.prepare(&sql)?;
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = values
            .iter()
            .map(|value| value as &dyn rusqlite::ToSql)
            .collect();
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let offset_i64 = i64::try_from(offset).unwrap_or(i64::MAX);
        params_vec.push(&limit_i64);
        params_vec.push(&offset_i64);
        let rows = stmt.query_map(params_vec.as_slice(), |row| {
            Ok(LlmSpendMetric {
                bucket: row.get(0)?,
                tenant: row.get(1)?,
                agent: row.get(2)?,
                user: row.get(3)?,
                project: row.get(4)?,
                provider: row.get(5)?,
                model: row.get(6)?,
                request_count: nonnegative_u64(row.get::<_, i64>(7)?),
                streaming_count: nonnegative_u64(row.get::<_, i64>(8)?),
                input_tokens: nonnegative_u64(row.get::<_, i64>(9)?),
                output_tokens: nonnegative_u64(row.get::<_, i64>(10)?),
                total_tokens: nonnegative_u64(row.get::<_, i64>(11)?),
                last_request: row.get(12)?,
            })
        })?;
        let metrics: Result<Vec<_>, _> = rows.collect();
        let metrics = metrics?;
        let next_offset = (metrics.len() == limit).then_some(offset.saturating_add(limit));
        Ok(LlmSpendReport {
            metrics,
            next_offset,
            retention_days: RETENTION_DAYS,
            monetary_cost: MonetaryCostStatus::default(),
        })
    }
}

fn nonnegative_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn bounded_dimension(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "unknown".into();
    }
    trimmed.chars().take(MAX_DIMENSION_LENGTH).collect()
}

fn event_bucket(timestamp: &str) -> Result<String> {
    let parsed = DateTime::parse_from_rfc3339(timestamp)
        .with_context(|| format!("invalid LLM event timestamp '{}'", timestamp))?;
    Ok(parsed.with_timezone(&Utc).format("%Y-%m-%d").to_string())
}

fn parse_bound(value: Option<&str>, name: &str) -> Result<Option<String>> {
    let Some(value) = value else { return Ok(None) };
    let parsed = if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        date
    } else {
        DateTime::parse_from_rfc3339(value)
            .with_context(|| format!("invalid {name} bound '{value}'"))?
            .with_timezone(&Utc)
            .date_naive()
    };
    Ok(Some(parsed.format("%Y-%m-%d").to_string()))
}

static GLOBAL_STORE: OnceLock<Option<LlmSpendStore>> = OnceLock::new();

/// Return the process-wide durable store. A storage failure degrades to
/// in-memory compatibility mode; proxy traffic must remain available.
pub fn global_store() -> Option<&'static LlmSpendStore> {
    GLOBAL_STORE
        .get_or_init(|| match LlmSpendStore::open_default() {
            Ok(store) => Some(store),
            Err(error) => {
                eprintln!("[llm-spend] durable aggregation unavailable: {error}");
                None
            }
        })
        .as_ref()
}

/// Persist an intercepted event when durable storage is available.
pub fn record_event(event: &crate::llm_intercept::LlmEvent) -> Result<()> {
    if let Some(store) = global_store() {
        store.record_event(event)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn event(timestamp: &str) -> crate::llm_intercept::LlmEvent {
        crate::llm_intercept::LlmEvent {
            timestamp: timestamp.into(),
            sandbox: "sandbox".into(),
            provider: "openai".into(),
            host: "api.openai.com".into(),
            method: "POST".into(),
            path: "/v1/chat/completions".into(),
            model: Some("gpt-4o".into()),
            status: Some(200),
            latency_ms: Some(10),
            input_tokens: Some(3),
            output_tokens: Some(7),
            total_tokens: Some(10),
            streaming: false,
            secret_injected: true,
            key_source: "sandbox".into(),
            tenant: Some("acme".into()),
            agent: Some("codex".into()),
            user: Some("alice".into()),
            project: Some("agentkernel".into()),
        }
    }

    #[test]
    fn aggregates_by_day_and_identity() {
        let dir = tempdir().unwrap();
        let store = LlmSpendStore::new(dir.path().join("usage.db")).unwrap();
        store.record_event(&event("2026-08-23T00:00:00Z")).unwrap();
        store.record_event(&event("2026-08-23T12:00:00Z")).unwrap();
        let mut other_user = event("2026-08-23T13:00:00Z");
        other_user.user = Some("bob".into());
        store.record_event(&other_user).unwrap();
        let report = store
            .query(&LlmSpendFilter {
                tenant: Some("acme".into()),
                user: Some("alice".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(report.metrics.len(), 1);
        assert_eq!(report.metrics[0].request_count, 2);
        assert_eq!(report.metrics[0].total_tokens, 20);
        assert_eq!(report.metrics[0].user, "alice");
        assert!(!report.monetary_cost.available);
    }

    #[test]
    fn query_rejects_reversed_bounds() {
        let dir = tempdir().unwrap();
        let store = LlmSpendStore::new(dir.path().join("usage.db")).unwrap();
        let err = store
            .query(&LlmSpendFilter {
                from: Some("2026-08-24".into()),
                to: Some("2026-08-23".into()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap_err();
        assert!(err.to_string().contains("from must be earlier"));
    }

    #[test]
    fn query_rejects_invalid_pagination() {
        let dir = tempdir().unwrap();
        let store = LlmSpendStore::new(dir.path().join("usage.db")).unwrap();
        for filter in [
            LlmSpendFilter {
                limit: Some(0),
                ..Default::default()
            },
            LlmSpendFilter {
                limit: Some(MAX_PAGE_SIZE + 1),
                ..Default::default()
            },
            LlmSpendFilter {
                offset: Some(MAX_QUERY_OFFSET + 1),
                ..Default::default()
            },
        ] {
            assert!(store.query(&filter).is_err());
        }
    }

    #[test]
    fn daily_cardinality_uses_a_bounded_overflow_bucket() {
        let dir = tempdir().unwrap();
        let store = LlmSpendStore::new(dir.path().join("usage.db")).unwrap();
        let conn = store.connection().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        for index in 0..(MAX_DAILY_ROWS_PER_TENANT_USER - 1) {
            tx.execute(
                "INSERT INTO llm_usage_daily
                    (bucket, tenant, agent, user_id, project, provider, model,
                     request_count, streaming_count, input_tokens, output_tokens,
                     total_tokens, last_request)
                 VALUES ('2026-08-23', 'acme', ?1, 'alice', 'project',
                         'openai', 'gpt-4o', 1, 0, 1, 1, 2,
                         '2026-08-23T00:00:00Z')",
                [format!("agent-{index}")],
            )
            .unwrap();
        }
        tx.commit().unwrap();

        store.record_event(&event("2026-08-23T12:00:00Z")).unwrap();
        let mut bob_event = event("2026-08-23T13:00:00Z");
        bob_event.user = Some("bob".into());
        store.record_event(&bob_event).unwrap();
        let report = store
            .query(&LlmSpendFilter {
                tenant: Some("acme".into()),
                project: Some(OVERFLOW_DIMENSION.into()),
                limit: Some(MAX_PAGE_SIZE),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(report.metrics.len(), 1);
        assert_eq!(report.metrics[0].agent, OVERFLOW_DIMENSION);
        assert_eq!(report.metrics[0].user, "alice");
        assert_eq!(report.metrics[0].total_tokens, 10);

        let bob_report = store
            .query(&LlmSpendFilter {
                tenant: Some("acme".into()),
                user: Some("bob".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(bob_report.metrics.len(), 1);
        assert_eq!(bob_report.metrics[0].agent, "codex");
        assert_eq!(bob_report.metrics[0].total_tokens, 10);

        let conn = store.connection().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM llm_usage_daily
                 WHERE bucket = '2026-08-23' AND tenant = 'acme' AND user_id = 'alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count <= MAX_DAILY_ROWS_PER_TENANT_USER);
    }

    #[test]
    fn ingests_legacy_event_jsonl_without_prompt_data() {
        let dir = tempdir().unwrap();
        let store = LlmSpendStore::new(dir.path().join("usage.db")).unwrap();
        let log = dir.path().join("events.jsonl");
        std::fs::write(
            &log,
            serde_json::to_string(&event("2026-08-23T00:00:00Z")).unwrap(),
        )
        .unwrap();
        assert_eq!(store.ingest_jsonl(&log).unwrap(), 1);
        assert_eq!(store.query(&Default::default()).unwrap().metrics.len(), 1);
    }
}
