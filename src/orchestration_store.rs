//! Durable orchestration persistence types and CRUD operations.

use crate::durable_storage::DurableStorage;
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};

/// Lifecycle state for durable orchestrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Terminated,
}

impl std::fmt::Display for OrchestrationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrchestrationStatus::Pending => write!(f, "pending"),
            OrchestrationStatus::Running => write!(f, "running"),
            OrchestrationStatus::Completed => write!(f, "completed"),
            OrchestrationStatus::Failed => write!(f, "failed"),
            OrchestrationStatus::Terminated => write!(f, "terminated"),
        }
    }
}

impl std::str::FromStr for OrchestrationStatus {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "terminated" => Ok(Self::Terminated),
            other => Err(format!("invalid orchestration status '{}'", other)),
        }
    }
}

/// Persisted orchestration record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationRecord {
    pub id: String,
    pub name: String,
    pub status: OrchestrationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Append-only orchestration history event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationEvent {
    pub sequence: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    pub timestamp: String,
}

/// Persisted orchestration definition payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationDefinition {
    pub name: String,
    pub definition: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

/// Backing engine for a durable store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DurableStoreKind {
    Sqlite,
    Postgres,
    Mysql,
    Redis,
}

impl std::fmt::Display for DurableStoreKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DurableStoreKind::Sqlite => write!(f, "sqlite"),
            DurableStoreKind::Postgres => write!(f, "postgres"),
            DurableStoreKind::Mysql => write!(f, "mysql"),
            DurableStoreKind::Redis => write!(f, "redis"),
        }
    }
}

impl std::str::FromStr for DurableStoreKind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "sqlite" => Ok(Self::Sqlite),
            "postgres" => Ok(Self::Postgres),
            "mysql" => Ok(Self::Mysql),
            "redis" => Ok(Self::Redis),
            other => Err(format!("invalid durable store kind '{other}'")),
        }
    }
}

/// Persisted durable store metadata record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableStoreRecord {
    pub id: String,
    pub name: String,
    pub kind: DurableStoreKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    pub config: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

/// Create request for durable store metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDurableStore {
    pub name: String,
    pub kind: DurableStoreKind,
    #[serde(default)]
    pub sandbox: Option<String>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
}

/// Query response payload for durable stores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableStoreQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<serde_json::Value>,
    pub row_count: usize,
}

/// Execute response payload for durable stores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableStoreExecuteResult {
    pub rows_affected: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_insert_rowid: Option<i64>,
}

/// Response payload for store command execution (for command-oriented engines).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableStoreCommandResult {
    pub result: serde_json::Value,
}

// --- Durable Object types ---

/// Lifecycle state for a durable object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableObjectStatus {
    Active,
    Hibernating,
    Deleted,
}

impl std::fmt::Display for DurableObjectStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Hibernating => write!(f, "hibernating"),
            Self::Deleted => write!(f, "deleted"),
        }
    }
}

impl std::str::FromStr for DurableObjectStatus {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "hibernating" => Ok(Self::Hibernating),
            "deleted" => Ok(Self::Deleted),
            other => Err(format!("invalid object status '{other}'")),
        }
    }
}

/// Persisted durable object record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableObjectRecord {
    pub id: String,
    pub class: String,
    pub object_id: String,
    pub status: DurableObjectStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    #[serde(default)]
    pub storage: serde_json::Value,
    pub idle_timeout_seconds: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Create request for a durable object.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateDurableObject {
    pub class: String,
    pub object_id: String,
    #[serde(default)]
    pub sandbox: Option<String>,
    #[serde(default)]
    pub storage: Option<serde_json::Value>,
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_seconds: i64,
}

fn default_idle_timeout() -> i64 {
    300
}

// --- Schedule types ---

/// Lifecycle state for a schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStatus {
    Active,
    Paused,
    Completed,
}

impl std::fmt::Display for ScheduleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Paused => write!(f, "paused"),
            Self::Completed => write!(f, "completed"),
        }
    }
}

impl std::str::FromStr for ScheduleStatus {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            other => Err(format!("invalid schedule status '{other}'")),
        }
    }
}

/// Persisted schedule record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRecord {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fire_at: Option<String>,
    pub method: String,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_orchestration: Option<String>,
    pub status: ScheduleStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fired_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Create request for a schedule.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateSchedule {
    pub name: String,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub fire_at: Option<String>,
    pub method: String,
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    #[serde(default)]
    pub target_class: Option<String>,
    #[serde(default)]
    pub target_object_id: Option<String>,
    #[serde(default)]
    pub target_orchestration: Option<String>,
}

/// Create request for orchestration persistence.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateOrchestration {
    pub name: String,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
}

/// Partial update request for orchestration persistence.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateOrchestration {
    #[serde(default)]
    pub status: Option<OrchestrationStatus>,
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
}

/// SQLite-backed orchestration store.
#[derive(Debug, Clone)]
pub struct OrchestrationStore {
    storage: DurableStorage,
}

impl OrchestrationStore {
    pub fn new(storage: DurableStorage) -> Self {
        Self { storage }
    }

    pub fn open_default() -> Result<Self> {
        Ok(Self::new(DurableStorage::open_default()?))
    }

    pub fn create(&self, req: CreateOrchestration) -> Result<OrchestrationRecord> {
        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::now_v7().to_string();
        let name = req.name;
        let input = req.input;
        let input_json = input.as_ref().map(serde_json::to_string).transpose()?;
        let status = OrchestrationStatus::Pending;
        let start_event = serde_json::json!({ "input": input.clone() });

        let conn = self.storage.open_connection()?;
        conn.execute(
            r#"
INSERT INTO orchestrations (
    id, name, status, input_json, output_json, error, created_at, updated_at
) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6)
"#,
            params![id, name, status.to_string(), input_json, now, now],
        )
        .context("failed to create orchestration record")?;

        self.append_event(&id, "OrchestratorStarted", start_event)
            .context("failed to append orchestrator started event")?;

        Ok(OrchestrationRecord {
            id,
            name,
            status,
            input,
            output: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn get(&self, id: &str) -> Result<Option<OrchestrationRecord>> {
        let conn = self.storage.open_connection()?;
        conn.query_row(
            r#"
SELECT id, name, status, input_json, output_json, error, created_at, updated_at
FROM orchestrations
WHERE id = ?1
"#,
            [id],
            Self::row_to_orchestration,
        )
        .optional()
        .context("failed to query orchestration by id")
    }

    pub fn list(&self, limit: usize, offset: usize) -> Result<Vec<OrchestrationRecord>> {
        let conn = self.storage.open_connection()?;
        let mut stmt = conn
            .prepare(
                r#"
SELECT id, name, status, input_json, output_json, error, created_at, updated_at
FROM orchestrations
ORDER BY created_at DESC
LIMIT ?1 OFFSET ?2
"#,
            )
            .context("failed to prepare list orchestrations query")?;

        let rows = stmt
            .query_map(
                params![limit as i64, offset as i64],
                Self::row_to_orchestration,
            )
            .context("failed to execute list orchestrations query")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("failed to parse orchestration row")?);
        }
        Ok(out)
    }

    pub fn append_event(
        &self,
        orchestration_id: &str,
        event_type: &str,
        event_data: serde_json::Value,
    ) -> Result<OrchestrationEvent> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let mut conn = self.storage.open_connection()?;
        let tx = conn
            .transaction()
            .context("failed to start event append transaction")?;

        let sequence: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM events WHERE orchestration_id = ?1",
                [orchestration_id],
                |row| row.get(0),
            )
            .context("failed to compute next event sequence")?;

        tx.execute(
            r#"
INSERT INTO events(orchestration_id, sequence, event_type, event_data, timestamp)
VALUES (?1, ?2, ?3, ?4, ?5)
"#,
            params![
                orchestration_id,
                sequence,
                event_type,
                serde_json::to_string(&event_data)?,
                timestamp,
            ],
        )
        .context("failed to append orchestration event")?;

        tx.execute(
            "UPDATE orchestrations SET updated_at = ?2 WHERE id = ?1",
            params![orchestration_id, timestamp],
        )
        .context("failed to update orchestration timestamp after event append")?;

        tx.commit()
            .context("failed to commit event append transaction")?;

        Ok(OrchestrationEvent {
            sequence,
            event_type: event_type.to_string(),
            data: Some(event_data),
            timestamp,
        })
    }

    pub fn list_events(
        &self,
        orchestration_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<OrchestrationEvent>> {
        let conn = self.storage.open_connection()?;
        let mut stmt = conn
            .prepare(
                r#"
SELECT sequence, event_type, event_data, timestamp
FROM events
WHERE orchestration_id = ?1
ORDER BY sequence ASC
LIMIT ?2 OFFSET ?3
"#,
            )
            .context("failed to prepare list events query")?;

        let rows = stmt
            .query_map(
                params![orchestration_id, limit as i64, offset as i64],
                Self::row_to_event,
            )
            .context("failed to execute list events query")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("failed to parse event row")?);
        }
        Ok(out)
    }

    pub fn upsert_definition(
        &self,
        name: &str,
        definition: serde_json::Value,
    ) -> Result<OrchestrationDefinition> {
        let now = chrono::Utc::now().to_rfc3339();
        let definition_json = serde_json::to_string(&definition)?;
        let conn = self.storage.open_connection()?;
        conn.execute(
            r#"
INSERT INTO orchestration_definitions(name, definition_json, created_at, updated_at)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(name) DO UPDATE SET
    definition_json = excluded.definition_json,
    updated_at = excluded.updated_at
"#,
            params![name, definition_json, now, now],
        )
        .context("failed to upsert orchestration definition")?;

        self.get_definition(name)?
            .context("definition missing after upsert")
    }

    pub fn get_definition(&self, name: &str) -> Result<Option<OrchestrationDefinition>> {
        let conn = self.storage.open_connection()?;
        conn.query_row(
            r#"
SELECT name, definition_json, created_at, updated_at
FROM orchestration_definitions
WHERE name = ?1
"#,
            [name],
            Self::row_to_definition,
        )
        .optional()
        .context("failed to get orchestration definition")
    }

    pub fn list_definitions(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<OrchestrationDefinition>> {
        let conn = self.storage.open_connection()?;
        let mut stmt = conn
            .prepare(
                r#"
SELECT name, definition_json, created_at, updated_at
FROM orchestration_definitions
ORDER BY name ASC
LIMIT ?1 OFFSET ?2
"#,
            )
            .context("failed to prepare list definitions query")?;

        let rows = stmt
            .query_map(
                params![limit as i64, offset as i64],
                Self::row_to_definition,
            )
            .context("failed to execute list definitions query")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("failed to parse definition row")?);
        }
        Ok(out)
    }

    pub fn delete_definition(&self, name: &str) -> Result<bool> {
        let conn = self.storage.open_connection()?;
        let deleted = conn
            .execute(
                "DELETE FROM orchestration_definitions WHERE name = ?1",
                [name],
            )
            .context("failed to delete orchestration definition")?;
        Ok(deleted > 0)
    }

    pub fn create_store(&self, req: CreateDurableStore) -> Result<DurableStoreRecord> {
        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::now_v7().to_string();
        let name = req.name.trim().to_string();
        let kind = req.kind;
        let sandbox = req.sandbox;
        let mut config = req.config.unwrap_or_else(|| serde_json::json!({}));

        if kind == DurableStoreKind::Sqlite && config.get("path").is_none() {
            config = serde_json::json!({
                "path": Self::default_sqlite_store_path(&id).to_string_lossy(),
            });
        }

        let config_json = serde_json::to_string(&config)?;
        let conn = self.storage.open_connection()?;
        conn.execute(
            r#"
INSERT INTO stores(id, name, kind, sandbox, config_json, created_at, updated_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
"#,
            params![id, name, kind.to_string(), sandbox, config_json, now, now],
        )
        .context("failed to create durable store metadata")?;

        self.get_store(&id)?.context("store missing after create")
    }

    pub fn get_store(&self, id: &str) -> Result<Option<DurableStoreRecord>> {
        let conn = self.storage.open_connection()?;
        conn.query_row(
            r#"
SELECT id, name, kind, sandbox, config_json, created_at, updated_at
FROM stores
WHERE id = ?1
"#,
            [id],
            Self::row_to_store,
        )
        .optional()
        .context("failed to get durable store")
    }

    pub fn list_stores(&self, limit: usize, offset: usize) -> Result<Vec<DurableStoreRecord>> {
        let conn = self.storage.open_connection()?;
        let mut stmt = conn
            .prepare(
                r#"
SELECT id, name, kind, sandbox, config_json, created_at, updated_at
FROM stores
ORDER BY created_at DESC
LIMIT ?1 OFFSET ?2
"#,
            )
            .context("failed to prepare list stores query")?;

        let rows = stmt
            .query_map(params![limit as i64, offset as i64], Self::row_to_store)
            .context("failed to execute list stores query")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("failed to parse store row")?);
        }
        Ok(out)
    }

    pub fn delete_store(&self, id: &str) -> Result<bool> {
        let conn = self.storage.open_connection()?;
        let deleted = conn
            .execute("DELETE FROM stores WHERE id = ?1", [id])
            .context("failed to delete durable store")?;
        Ok(deleted > 0)
    }

    pub fn query_store(
        &self,
        id: &str,
        sql: &str,
        params: Vec<serde_json::Value>,
    ) -> Result<Option<DurableStoreQueryResult>> {
        let Some(store) = self.get_store(id)? else {
            return Ok(None);
        };

        match store.kind {
            DurableStoreKind::Sqlite => self.query_store_sqlite(&store, sql, params),
            DurableStoreKind::Postgres => tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(Self::query_store_postgres(&store, sql, params))
            }),
            DurableStoreKind::Mysql => tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(Self::query_store_mysql(&store, sql, params))
            }),
            DurableStoreKind::Redis => {
                anyhow::bail!("redis stores do not support SQL queries; use the command endpoint")
            }
        }
    }

    fn query_store_sqlite(
        &self,
        store: &DurableStoreRecord,
        sql: &str,
        params: Vec<serde_json::Value>,
    ) -> Result<Option<DurableStoreQueryResult>> {
        let conn = self.connect_durable_store(store)?;
        let mut stmt = conn
            .prepare(sql)
            .with_context(|| format!("failed preparing store query for store {}", store.id))?;
        let column_names: Vec<String> = stmt
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        let sql_params: Vec<rusqlite::types::Value> =
            params.into_iter().map(Self::json_to_sql_value).collect();
        let mut rows = stmt
            .query(params_from_iter(sql_params.iter()))
            .context("failed executing store query")?;

        let mut out_rows = Vec::new();
        while let Some(row) = rows.next().context("failed reading store query row")? {
            let mut object = serde_json::Map::new();
            for (idx, col) in column_names.iter().enumerate() {
                let value_ref = row
                    .get_ref(idx)
                    .with_context(|| format!("failed to read column {idx}"))?;
                object.insert(col.clone(), Self::sql_value_ref_to_json(value_ref));
            }
            out_rows.push(serde_json::Value::Object(object));
        }

        Ok(Some(DurableStoreQueryResult {
            columns: column_names,
            row_count: out_rows.len(),
            rows: out_rows,
        }))
    }

    async fn query_store_postgres(
        store: &DurableStoreRecord,
        sql: &str,
        _params: Vec<serde_json::Value>,
    ) -> Result<Option<DurableStoreQueryResult>> {
        let conn_str = Self::postgres_connection_string(store);
        let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
            .await
            .context("failed to connect to postgres store")?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("[durable-store] postgres connection error: {e}");
            }
        });

        let rows = client
            .query(sql, &[])
            .await
            .context("postgres query failed")?;

        let columns: Vec<String> = rows
            .first()
            .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
            .unwrap_or_default();

        let mut out_rows = Vec::new();
        for row in &rows {
            let mut object = serde_json::Map::new();
            for (idx, col) in row.columns().iter().enumerate() {
                let val = Self::pg_value_to_json(row, idx);
                object.insert(col.name().to_string(), val);
            }
            out_rows.push(serde_json::Value::Object(object));
        }

        Ok(Some(DurableStoreQueryResult {
            columns,
            row_count: out_rows.len(),
            rows: out_rows,
        }))
    }

    async fn query_store_mysql(
        store: &DurableStoreRecord,
        sql: &str,
        _params: Vec<serde_json::Value>,
    ) -> Result<Option<DurableStoreQueryResult>> {
        use mysql_async::prelude::*;

        let url = Self::mysql_connection_string(store);
        let pool = mysql_async::Pool::new(url.as_str());
        let mut conn = pool
            .get_conn()
            .await
            .context("failed to connect to mysql store")?;

        let result: Vec<mysql_async::Row> = conn.query(sql).await.context("mysql query failed")?;

        let columns: Vec<String> = result
            .first()
            .map(|r| {
                r.columns_ref()
                    .iter()
                    .map(|c| c.name_str().to_string())
                    .collect()
            })
            .unwrap_or_default();

        let mut out_rows = Vec::new();
        for row in &result {
            let mut object = serde_json::Map::new();
            for (idx, col_name) in columns.iter().enumerate() {
                let val: mysql_async::Value = row.get(idx).unwrap_or(mysql_async::Value::NULL);
                object.insert(col_name.clone(), Self::mysql_value_to_json(val));
            }
            out_rows.push(serde_json::Value::Object(object));
        }

        drop(conn);
        pool.disconnect().await.ok();

        Ok(Some(DurableStoreQueryResult {
            columns,
            row_count: out_rows.len(),
            rows: out_rows,
        }))
    }

    pub fn execute_store(
        &self,
        id: &str,
        sql: &str,
        params: Vec<serde_json::Value>,
    ) -> Result<Option<DurableStoreExecuteResult>> {
        let Some(store) = self.get_store(id)? else {
            return Ok(None);
        };

        match store.kind {
            DurableStoreKind::Sqlite => self.execute_store_sqlite(&store, sql, params),
            DurableStoreKind::Postgres => tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(Self::execute_store_postgres(&store, sql))
            }),
            DurableStoreKind::Mysql => tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(Self::execute_store_mysql(&store, sql))
            }),
            DurableStoreKind::Redis => {
                anyhow::bail!("redis stores do not support SQL execute; use the command endpoint")
            }
        }
    }

    fn execute_store_sqlite(
        &self,
        store: &DurableStoreRecord,
        sql: &str,
        params: Vec<serde_json::Value>,
    ) -> Result<Option<DurableStoreExecuteResult>> {
        let conn = self.connect_durable_store(store)?;
        let sql_params: Vec<rusqlite::types::Value> =
            params.into_iter().map(Self::json_to_sql_value).collect();
        let rows_affected = conn
            .execute(sql, params_from_iter(sql_params.iter()))
            .context("failed executing store statement")?;

        Ok(Some(DurableStoreExecuteResult {
            rows_affected,
            last_insert_rowid: Some(conn.last_insert_rowid()),
        }))
    }

    pub fn command_store(
        &self,
        id: &str,
        command: Vec<String>,
    ) -> Result<Option<DurableStoreCommandResult>> {
        let Some(store) = self.get_store(id)? else {
            return Ok(None);
        };

        if command.is_empty() {
            anyhow::bail!("command must not be empty");
        }

        match store.kind {
            DurableStoreKind::Redis => {
                let result = Self::execute_redis_command(&store, command)?;
                Ok(Some(result))
            }
            DurableStoreKind::Sqlite | DurableStoreKind::Postgres | DurableStoreKind::Mysql => {
                anyhow::bail!(
                    "store kind '{}' does not support the command endpoint; use query or execute",
                    store.kind
                )
            }
        }
    }

    // --- Durable Object CRUD ---

    pub fn create_object(&self, req: CreateDurableObject) -> Result<DurableObjectRecord> {
        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::now_v7().to_string();
        let storage = req.storage.unwrap_or_else(|| serde_json::json!({}));
        let storage_json = serde_json::to_string(&storage)?;
        let status = DurableObjectStatus::Hibernating;

        let conn = self.storage.open_connection()?;
        conn.execute(
            r#"
INSERT INTO objects(id, class, object_id, status, sandbox, storage_json, idle_timeout_seconds, created_at, updated_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
"#,
            params![
                id,
                req.class,
                req.object_id,
                status.to_string(),
                req.sandbox,
                storage_json,
                req.idle_timeout_seconds,
                now,
                now,
            ],
        )
        .context("failed to create durable object")?;

        Ok(DurableObjectRecord {
            id,
            class: req.class,
            object_id: req.object_id,
            status,
            sandbox: req.sandbox,
            storage,
            idle_timeout_seconds: req.idle_timeout_seconds,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn get_object(&self, id: &str) -> Result<Option<DurableObjectRecord>> {
        let conn = self.storage.open_connection()?;
        conn.query_row(
            r#"
SELECT id, class, object_id, status, sandbox, storage_json, idle_timeout_seconds, created_at, updated_at
FROM objects WHERE id = ?1
"#,
            [id],
            Self::row_to_object,
        )
        .optional()
        .context("failed to get durable object")
    }

    pub fn list_objects(&self, limit: usize, offset: usize) -> Result<Vec<DurableObjectRecord>> {
        let conn = self.storage.open_connection()?;
        let mut stmt = conn
            .prepare(
                r#"
SELECT id, class, object_id, status, sandbox, storage_json, idle_timeout_seconds, created_at, updated_at
FROM objects
WHERE status != 'deleted'
ORDER BY created_at DESC
LIMIT ?1 OFFSET ?2
"#,
            )
            .context("failed to prepare list objects query")?;

        let rows = stmt
            .query_map(params![limit as i64, offset as i64], Self::row_to_object)
            .context("failed to execute list objects query")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("failed to parse object row")?);
        }
        Ok(out)
    }

    pub fn delete_object(&self, id: &str) -> Result<bool> {
        let conn = self.storage.open_connection()?;
        let deleted = conn
            .execute("DELETE FROM objects WHERE id = ?1", [id])
            .context("failed to delete durable object")?;
        Ok(deleted > 0)
    }

    /// Find a durable object by class name and object_id.
    pub fn find_object_by_class_and_id(
        &self,
        class: &str,
        object_id: &str,
    ) -> Result<Option<DurableObjectRecord>> {
        let conn = self.storage.open_connection()?;
        conn.query_row(
            r#"
SELECT id, class, object_id, status, sandbox, storage_json, idle_timeout_seconds, created_at, updated_at
FROM objects
WHERE class = ?1 AND object_id = ?2 AND status != 'deleted'
"#,
            params![class, object_id],
            Self::row_to_object,
        )
        .optional()
        .context("failed to find object by class and id")
    }

    /// Update the status and optional sandbox name of a durable object.
    pub fn update_object_status(
        &self,
        id: &str,
        status: DurableObjectStatus,
        sandbox: Option<&str>,
    ) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.storage.open_connection()?;
        let updated = conn
            .execute(
                "UPDATE objects SET status = ?2, sandbox = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, status.to_string(), sandbox, now],
            )
            .context("failed to update object status")?;
        Ok(updated > 0)
    }

    /// Persist storage JSON for a durable object.
    pub fn update_object_storage(&self, id: &str, storage: &serde_json::Value) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let storage_json = serde_json::to_string(storage)?;
        let conn = self.storage.open_connection()?;
        let updated = conn
            .execute(
                "UPDATE objects SET storage_json = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, storage_json, now],
            )
            .context("failed to update object storage")?;
        Ok(updated > 0)
    }

    /// List all active (non-deleted, non-hibernating) objects.
    pub fn list_active_objects(&self) -> Result<Vec<DurableObjectRecord>> {
        let conn = self.storage.open_connection()?;
        let mut stmt = conn
            .prepare(
                r#"
SELECT id, class, object_id, status, sandbox, storage_json, idle_timeout_seconds, created_at, updated_at
FROM objects
WHERE status = 'active'
ORDER BY updated_at ASC
"#,
            )
            .context("failed to prepare list active objects query")?;

        let rows = stmt
            .query_map([], Self::row_to_object)
            .context("failed to execute list active objects query")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("failed to parse active object row")?);
        }
        Ok(out)
    }

    /// Touch the updated_at timestamp of an object (to reset idle timer).
    pub fn touch_object(&self, id: &str) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.storage.open_connection()?;
        let updated = conn
            .execute(
                "UPDATE objects SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .context("failed to touch object timestamp")?;
        Ok(updated > 0)
    }

    // --- Schedule CRUD ---

    pub fn create_schedule(&self, req: CreateSchedule) -> Result<ScheduleRecord> {
        if req.cron.is_none() && req.fire_at.is_none() {
            anyhow::bail!("schedule must have either 'cron' or 'fire_at'");
        }

        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::now_v7().to_string();
        let args = req.args.unwrap_or_else(|| serde_json::json!({}));
        let args_json = serde_json::to_string(&args)?;
        let status = ScheduleStatus::Active;

        let conn = self.storage.open_connection()?;
        conn.execute(
            r#"
INSERT INTO schedules(id, name, cron, fire_at, method, args_json, target_class, target_object_id, target_orchestration, status, created_at, updated_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
"#,
            params![
                id,
                req.name,
                req.cron,
                req.fire_at,
                req.method,
                args_json,
                req.target_class,
                req.target_object_id,
                req.target_orchestration,
                status.to_string(),
                now,
                now,
            ],
        )
        .context("failed to create schedule")?;

        Ok(ScheduleRecord {
            id,
            name: req.name,
            cron: req.cron,
            fire_at: req.fire_at,
            method: req.method,
            args,
            target_class: req.target_class,
            target_object_id: req.target_object_id,
            target_orchestration: req.target_orchestration,
            status,
            last_fired_at: None,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn get_schedule(&self, id: &str) -> Result<Option<ScheduleRecord>> {
        let conn = self.storage.open_connection()?;
        conn.query_row(
            r#"
SELECT id, name, cron, fire_at, method, args_json, target_class, target_object_id, target_orchestration, status, last_fired_at, created_at, updated_at
FROM schedules WHERE id = ?1
"#,
            [id],
            Self::row_to_schedule,
        )
        .optional()
        .context("failed to get schedule")
    }

    pub fn list_schedules(&self, limit: usize, offset: usize) -> Result<Vec<ScheduleRecord>> {
        let conn = self.storage.open_connection()?;
        let mut stmt = conn
            .prepare(
                r#"
SELECT id, name, cron, fire_at, method, args_json, target_class, target_object_id, target_orchestration, status, last_fired_at, created_at, updated_at
FROM schedules
ORDER BY created_at DESC
LIMIT ?1 OFFSET ?2
"#,
            )
            .context("failed to prepare list schedules query")?;

        let rows = stmt
            .query_map(params![limit as i64, offset as i64], Self::row_to_schedule)
            .context("failed to execute list schedules query")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("failed to parse schedule row")?);
        }
        Ok(out)
    }

    pub fn delete_schedule(&self, id: &str) -> Result<bool> {
        let conn = self.storage.open_connection()?;
        let deleted = conn
            .execute("DELETE FROM schedules WHERE id = ?1", [id])
            .context("failed to delete schedule")?;
        Ok(deleted > 0)
    }

    /// Mark a schedule as just fired (update last_fired_at to now).
    pub fn mark_schedule_fired(&self, id: &str) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.storage.open_connection()?;
        let updated = conn
            .execute(
                "UPDATE schedules SET last_fired_at = ?2, updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .context("failed to mark schedule as fired")?;
        Ok(updated > 0)
    }

    fn execute_redis_command(
        store: &DurableStoreRecord,
        command: Vec<String>,
    ) -> Result<DurableStoreCommandResult> {
        let host = store
            .config
            .get("host")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("127.0.0.1");
        let port = store
            .config
            .get("port")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(6379) as u16;
        let db = store
            .config
            .get("db")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as i64;
        let password = store
            .config
            .get("password")
            .and_then(serde_json::Value::as_str);

        let mut url = format!("redis://{host}:{port}/{db}");
        if let Some(pw) = password {
            url = format!("redis://:{pw}@{host}:{port}/{db}");
        }

        let client = redis::Client::open(url.as_str()).context("failed to create redis client")?;
        let mut conn = client
            .get_connection()
            .context("failed to connect to redis")?;

        let cmd_name = &command[0];
        let args = &command[1..];
        let mut cmd = redis::cmd(cmd_name);
        for arg in args {
            cmd.arg(arg);
        }

        let raw: redis::Value = cmd
            .query(&mut conn)
            .with_context(|| format!("redis command '{}' failed", cmd_name))?;

        Ok(DurableStoreCommandResult {
            result: redis_value_to_json(raw),
        })
    }

    // ---- Postgres helpers ----

    fn postgres_connection_string(store: &DurableStoreRecord) -> String {
        let host = store
            .config
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("127.0.0.1");
        let port = store
            .config
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(5432);
        let user = store
            .config
            .get("user")
            .and_then(|v| v.as_str())
            .unwrap_or("postgres");
        let password = store
            .config
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let dbname = store
            .config
            .get("dbname")
            .and_then(|v| v.as_str())
            .unwrap_or("postgres");
        format!("host={host} port={port} user={user} password={password} dbname={dbname}")
    }

    async fn execute_store_postgres(
        store: &DurableStoreRecord,
        sql: &str,
    ) -> Result<Option<DurableStoreExecuteResult>> {
        let conn_str = Self::postgres_connection_string(store);
        let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
            .await
            .context("failed to connect to postgres store")?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("[durable-store] postgres connection error: {e}");
            }
        });

        let rows_affected = client
            .execute(sql, &[])
            .await
            .context("postgres execute failed")?;

        Ok(Some(DurableStoreExecuteResult {
            rows_affected: rows_affected as usize,
            last_insert_rowid: None,
        }))
    }

    fn pg_value_to_json(row: &tokio_postgres::Row, idx: usize) -> serde_json::Value {
        // Try common types in order
        if let Ok(v) = row.try_get::<_, bool>(idx) {
            return serde_json::Value::Bool(v);
        }
        if let Ok(v) = row.try_get::<_, i32>(idx) {
            return serde_json::json!(v);
        }
        if let Ok(v) = row.try_get::<_, i64>(idx) {
            return serde_json::json!(v);
        }
        if let Ok(v) = row.try_get::<_, f64>(idx) {
            return serde_json::json!(v);
        }
        if let Ok(v) = row.try_get::<_, String>(idx) {
            return serde_json::Value::String(v);
        }
        if let Ok(v) = row.try_get::<_, serde_json::Value>(idx) {
            return v;
        }
        serde_json::Value::Null
    }

    // ---- MySQL helpers ----

    fn mysql_connection_string(store: &DurableStoreRecord) -> String {
        let host = store
            .config
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("127.0.0.1");
        let port = store
            .config
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(3306);
        let user = store
            .config
            .get("user")
            .and_then(|v| v.as_str())
            .unwrap_or("root");
        let password = store
            .config
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let dbname = store
            .config
            .get("dbname")
            .and_then(|v| v.as_str())
            .unwrap_or("mysql");
        format!("mysql://{user}:{password}@{host}:{port}/{dbname}")
    }

    async fn execute_store_mysql(
        store: &DurableStoreRecord,
        sql: &str,
    ) -> Result<Option<DurableStoreExecuteResult>> {
        use mysql_async::prelude::*;

        let url = Self::mysql_connection_string(store);
        let pool = mysql_async::Pool::new(url.as_str());
        let mut conn = pool
            .get_conn()
            .await
            .context("failed to connect to mysql store")?;

        conn.exec_drop(sql, ())
            .await
            .context("mysql execute failed")?;

        drop(conn);
        pool.disconnect().await.ok();

        Ok(Some(DurableStoreExecuteResult {
            rows_affected: 0,
            last_insert_rowid: None,
        }))
    }

    fn mysql_value_to_json(val: mysql_async::Value) -> serde_json::Value {
        match val {
            mysql_async::Value::NULL => serde_json::Value::Null,
            mysql_async::Value::Int(i) => serde_json::json!(i),
            mysql_async::Value::UInt(u) => serde_json::json!(u),
            mysql_async::Value::Float(f) => serde_json::json!(f),
            mysql_async::Value::Double(d) => serde_json::json!(d),
            mysql_async::Value::Bytes(b) => {
                serde_json::Value::String(String::from_utf8_lossy(&b).to_string())
            }
            mysql_async::Value::Date(y, m, d, h, mi, s, _us) => {
                serde_json::Value::String(format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}"))
            }
            mysql_async::Value::Time(neg, d, h, mi, s, _us) => {
                let sign = if neg { "-" } else { "" };
                serde_json::Value::String(format!("{sign}{d}d {h:02}:{mi:02}:{s:02}"))
            }
        }
    }

    pub fn update(
        &self,
        id: &str,
        patch: UpdateOrchestration,
    ) -> Result<Option<OrchestrationRecord>> {
        let Some(mut current) = self.get(id)? else {
            return Ok(None);
        };

        if let Some(status) = patch.status {
            current.status = status;
        }
        if let Some(output) = patch.output {
            current.output = Some(output);
        }
        if let Some(error) = patch.error {
            current.error = Some(error);
        }
        current.updated_at = chrono::Utc::now().to_rfc3339();

        let conn = self.storage.open_connection()?;
        conn.execute(
            r#"
UPDATE orchestrations
SET status = ?2,
    output_json = ?3,
    error = ?4,
    updated_at = ?5
WHERE id = ?1
"#,
            params![
                id,
                current.status.to_string(),
                current
                    .output
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                current.error,
                current.updated_at,
            ],
        )
        .context("failed to update orchestration record")?;

        Ok(Some(current))
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let conn = self.storage.open_connection()?;
        let deleted = conn
            .execute("DELETE FROM orchestrations WHERE id = ?1", [id])
            .context("failed to delete orchestration record")?;
        Ok(deleted > 0)
    }

    fn row_to_orchestration(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrchestrationRecord> {
        let status_raw: String = row.get(2)?;
        let input_json: Option<String> = row.get(3)?;
        let output_json: Option<String> = row.get(4)?;

        let status = status_raw.parse::<OrchestrationStatus>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?;

        let input = parse_json_field(input_json, 3)?;
        let output = parse_json_field(output_json, 4)?;

        Ok(OrchestrationRecord {
            id: row.get(0)?,
            name: row.get(1)?,
            status,
            input,
            output,
            error: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }

    fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrchestrationEvent> {
        let data_raw: String = row.get(2)?;
        let data = serde_json::from_str(&data_raw).map(Some).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(OrchestrationEvent {
            sequence: row.get(0)?,
            event_type: row.get(1)?,
            data,
            timestamp: row.get(3)?,
        })
    }

    fn row_to_definition(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrchestrationDefinition> {
        let definition_raw: String = row.get(1)?;
        let definition = serde_json::from_str(&definition_raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(OrchestrationDefinition {
            name: row.get(0)?,
            definition,
            created_at: row.get(2)?,
            updated_at: row.get(3)?,
        })
    }

    fn row_to_store(row: &rusqlite::Row<'_>) -> rusqlite::Result<DurableStoreRecord> {
        let kind_raw: String = row.get(2)?;
        let kind = kind_raw.parse::<DurableStoreKind>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?;
        let config_raw: String = row.get(4)?;
        let config = serde_json::from_str(&config_raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(DurableStoreRecord {
            id: row.get(0)?,
            name: row.get(1)?,
            kind,
            sandbox: row.get(3)?,
            config,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    }

    fn row_to_object(row: &rusqlite::Row<'_>) -> rusqlite::Result<DurableObjectRecord> {
        let status_raw: String = row.get(3)?;
        let status = status_raw.parse::<DurableObjectStatus>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?;
        let storage_raw: String = row.get(5)?;
        let storage = serde_json::from_str(&storage_raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(DurableObjectRecord {
            id: row.get(0)?,
            class: row.get(1)?,
            object_id: row.get(2)?,
            status,
            sandbox: row.get(4)?,
            storage,
            idle_timeout_seconds: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    }

    fn row_to_schedule(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduleRecord> {
        let status_raw: String = row.get(9)?;
        let status = status_raw.parse::<ScheduleStatus>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?;
        let args_raw: String = row.get(5)?;
        let args = serde_json::from_str(&args_raw).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?;

        Ok(ScheduleRecord {
            id: row.get(0)?,
            name: row.get(1)?,
            cron: row.get(2)?,
            fire_at: row.get(3)?,
            method: row.get(4)?,
            args,
            target_class: row.get(6)?,
            target_object_id: row.get(7)?,
            target_orchestration: row.get(8)?,
            status,
            last_fired_at: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    }

    fn connect_durable_store(&self, store: &DurableStoreRecord) -> Result<rusqlite::Connection> {
        match store.kind {
            DurableStoreKind::Sqlite => {
                let path = store
                    .config
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| Self::default_sqlite_store_path(&store.id));
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "failed to create sqlite store directory {}",
                            parent.display()
                        )
                    })?;
                }
                let conn = rusqlite::Connection::open(path)
                    .context("failed to open sqlite durable store")?;
                conn.execute_batch(
                    r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = ON;
"#,
                )
                .context("failed to apply sqlite store pragmas")?;
                Ok(conn)
            }
            DurableStoreKind::Postgres => {
                anyhow::bail!("postgres durable stores are not executable in this runtime yet")
            }
            DurableStoreKind::Mysql => {
                anyhow::bail!("mysql durable stores are not executable in this runtime yet")
            }
            DurableStoreKind::Redis => {
                anyhow::bail!("redis durable stores must use the command endpoint")
            }
        }
    }

    fn default_sqlite_store_path(id: &str) -> std::path::PathBuf {
        let durable_root = DurableStorage::default_db_path()
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/agentkernel/durable"));
        durable_root.join("stores").join(format!("{id}.db"))
    }

    fn json_to_sql_value(value: serde_json::Value) -> rusqlite::types::Value {
        use rusqlite::types::Value;

        match value {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(v) => Value::Integer(i64::from(v)),
            serde_json::Value::Number(n) => {
                if let Some(v) = n.as_i64() {
                    Value::Integer(v)
                } else if let Some(v) = n.as_u64() {
                    match i64::try_from(v) {
                        Ok(i) => Value::Integer(i),
                        Err(_) => Value::Real(v as f64),
                    }
                } else if let Some(v) = n.as_f64() {
                    Value::Real(v)
                } else {
                    Value::Null
                }
            }
            serde_json::Value::String(s) => Value::Text(s),
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                Value::Text(value.to_string())
            }
        }
    }

    fn sql_value_ref_to_json(value: rusqlite::types::ValueRef<'_>) -> serde_json::Value {
        use rusqlite::types::ValueRef;

        match value {
            ValueRef::Null => serde_json::Value::Null,
            ValueRef::Integer(v) => serde_json::Value::from(v),
            ValueRef::Real(v) => serde_json::Value::from(v),
            ValueRef::Text(v) => serde_json::Value::String(String::from_utf8_lossy(v).to_string()),
            ValueRef::Blob(v) => serde_json::Value::String(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                v,
            )),
        }
    }
}

fn redis_value_to_json(value: redis::Value) -> serde_json::Value {
    match value {
        redis::Value::Nil => serde_json::Value::Null,
        redis::Value::Int(v) => serde_json::Value::from(v),
        redis::Value::BulkString(bytes) => match String::from_utf8(bytes.clone()) {
            Ok(s) => serde_json::Value::String(s),
            Err(_) => serde_json::Value::String(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &bytes,
            )),
        },
        redis::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(redis_value_to_json).collect())
        }
        redis::Value::SimpleString(s) => serde_json::Value::String(s),
        redis::Value::Okay => serde_json::Value::String("OK".to_string()),
        redis::Value::Map(pairs) => {
            let mut map = serde_json::Map::new();
            for (k, v) in pairs {
                let key = match k {
                    redis::Value::BulkString(b) => {
                        String::from_utf8(b).unwrap_or_else(|_| "<binary>".to_string())
                    }
                    redis::Value::SimpleString(s) => s,
                    other => format!("{other:?}"),
                };
                map.insert(key, redis_value_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        redis::Value::Double(v) => serde_json::Value::from(v),
        redis::Value::Boolean(v) => serde_json::Value::Bool(v),
        redis::Value::VerbatimString { text, .. } => serde_json::Value::String(text),
        redis::Value::BigNumber(v) => serde_json::Value::String(v.to_string()),
        redis::Value::Set(items) => {
            serde_json::Value::Array(items.into_iter().map(redis_value_to_json).collect())
        }
        redis::Value::Attribute { data, .. } => redis_value_to_json(*data),
        redis::Value::Push { data, .. } => {
            serde_json::Value::Array(data.into_iter().map(redis_value_to_json).collect())
        }
        redis::Value::ServerError(e) => serde_json::json!({ "error": format!("{e:?}") }),
        other => serde_json::json!({ "unsupported": format!("{other:?}") }),
    }
}

fn parse_json_field(
    value: Option<String>,
    col: usize,
) -> rusqlite::Result<Option<serde_json::Value>> {
    match value {
        Some(raw) => serde_json::from_str(&raw).map(Some).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e))
        }),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_query_orchestration() {
        let temp = tempfile::TempDir::new().unwrap();
        let storage = DurableStorage::new(temp.path().join("orchestrations.db")).unwrap();
        let store = OrchestrationStore::new(storage);

        let created = store
            .create(CreateOrchestration {
                name: "test-orchestration".to_string(),
                input: Some(serde_json::json!({"step": 1})),
            })
            .unwrap();
        assert_eq!(created.name, "test-orchestration");
        assert_eq!(created.status, OrchestrationStatus::Pending);

        let fetched = store.get(&created.id).unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.input, Some(serde_json::json!({"step": 1})));

        let listed = store.list(10, 0).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let history = store.list_events(&created.id, 10, 0).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].sequence, 1);
        assert_eq!(history[0].event_type, "OrchestratorStarted");
    }

    #[test]
    fn test_append_and_list_events() {
        let temp = tempfile::TempDir::new().unwrap();
        let storage = DurableStorage::new(temp.path().join("orchestrations.db")).unwrap();
        let store = OrchestrationStore::new(storage);

        let created = store
            .create(CreateOrchestration {
                name: "signal-test".to_string(),
                input: None,
            })
            .unwrap();

        let event = store
            .append_event(
                &created.id,
                "EventRaised",
                serde_json::json!({
                    "name": "approval",
                    "data": { "approved": true }
                }),
            )
            .unwrap();
        assert_eq!(event.sequence, 2);
        assert_eq!(event.event_type, "EventRaised");

        let history = store.list_events(&created.id, 10, 0).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].event_type, "OrchestratorStarted");
        assert_eq!(history[1].event_type, "EventRaised");
    }

    #[test]
    fn test_upsert_and_list_definitions() {
        let temp = tempfile::TempDir::new().unwrap();
        let storage = DurableStorage::new(temp.path().join("orchestrations.db")).unwrap();
        let store = OrchestrationStore::new(storage);

        let first = store
            .upsert_definition(
                "deploy-pipeline",
                serde_json::json!({
                    "name": "deploy-pipeline",
                    "activities": [
                        {"name": "run-tests", "command": ["cargo", "test"]}
                    ]
                }),
            )
            .unwrap();
        assert_eq!(first.name, "deploy-pipeline");

        let second = store
            .upsert_definition(
                "deploy-pipeline",
                serde_json::json!({
                    "name": "deploy-pipeline",
                    "activities": [
                        {"name": "lint", "command": ["cargo", "clippy"]}
                    ]
                }),
            )
            .unwrap();
        assert_eq!(second.name, "deploy-pipeline");

        let fetched = store.get_definition("deploy-pipeline").unwrap().unwrap();
        assert_eq!(fetched.name, "deploy-pipeline");
        assert_eq!(
            fetched
                .definition
                .get("activities")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(1)
        );

        let listed = store.list_definitions(10, 0).unwrap();
        assert_eq!(listed.len(), 1);

        let deleted = store.delete_definition("deploy-pipeline").unwrap();
        assert!(deleted);
        assert!(store.get_definition("deploy-pipeline").unwrap().is_none());
    }

    #[test]
    fn test_create_query_execute_sqlite_store() {
        let temp = tempfile::TempDir::new().unwrap();
        let storage = DurableStorage::new(temp.path().join("orchestrations.db")).unwrap();
        let store = OrchestrationStore::new(storage);
        let sqlite_path = temp.path().join("store.db");

        let created = store
            .create_store(CreateDurableStore {
                name: "agent-state".to_string(),
                kind: DurableStoreKind::Sqlite,
                sandbox: Some("sandbox-a".to_string()),
                config: Some(serde_json::json!({
                    "path": sqlite_path.to_string_lossy()
                })),
            })
            .unwrap();
        assert_eq!(created.name, "agent-state");
        assert_eq!(created.kind, DurableStoreKind::Sqlite);

        store
            .execute_store(
                &created.id,
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                vec![],
            )
            .unwrap()
            .unwrap();

        let inserted = store
            .execute_store(
                &created.id,
                "INSERT INTO items(name) VALUES (?)",
                vec![serde_json::json!("alpha")],
            )
            .unwrap()
            .unwrap();
        assert_eq!(inserted.rows_affected, 1);

        let queried = store
            .query_store(
                &created.id,
                "SELECT id, name FROM items WHERE name = ?",
                vec![serde_json::json!("alpha")],
            )
            .unwrap()
            .unwrap();
        assert_eq!(queried.row_count, 1);
        assert_eq!(
            queried.rows[0]
                .get("name")
                .and_then(serde_json::Value::as_str),
            Some("alpha")
        );

        let listed = store.list_stores(10, 0).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let deleted = store.delete_store(&created.id).unwrap();
        assert!(deleted);
        assert!(store.get_store(&created.id).unwrap().is_none());
    }
}
