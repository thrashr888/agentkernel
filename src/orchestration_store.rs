//! Durable orchestration persistence types and CRUD operations.

use crate::durable_storage::DurableStorage;
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
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

    pub fn default() -> Result<Self> {
        Ok(Self::new(DurableStorage::default()?))
    }

    pub fn create(&self, req: CreateOrchestration) -> Result<OrchestrationRecord> {
        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::now_v7().to_string();
        let input_json = req.input.as_ref().map(serde_json::to_string).transpose()?;
        let status = OrchestrationStatus::Pending;

        let conn = self.storage.open_connection()?;
        conn.execute(
            r#"
INSERT INTO orchestrations (
    id, name, status, input_json, output_json, error, created_at, updated_at
) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6)
"#,
            params![id, req.name, status.to_string(), input_json, now, now],
        )
        .context("failed to create orchestration record")?;

        Ok(OrchestrationRecord {
            id,
            name: req.name,
            status,
            input: req.input,
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
    }
}
