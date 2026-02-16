//! Durable workflow storage bootstrap backed by SQLite.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

const MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        r#"
CREATE TABLE IF NOT EXISTS orchestrations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    input_json TEXT,
    output_json TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_orchestrations_status
    ON orchestrations(status);
CREATE INDEX IF NOT EXISTS idx_orchestrations_created_at
    ON orchestrations(created_at DESC);
"#,
    ),
    (
        2,
        r#"
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    orchestration_id TEXT NOT NULL REFERENCES orchestrations(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    event_data TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    UNIQUE(orchestration_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_events_orchestration_sequence
    ON events(orchestration_id, sequence);
"#,
    ),
    (
        3,
        r#"
CREATE TABLE IF NOT EXISTS orchestration_definitions (
    name TEXT PRIMARY KEY,
    definition_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#,
    ),
];

/// SQLite durable storage wrapper with schema bootstrap.
#[derive(Debug, Clone)]
pub struct DurableStorage {
    db_path: PathBuf,
}

impl DurableStorage {
    /// Create and bootstrap storage at a custom path.
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let storage = Self { db_path };
        storage.bootstrap()?;
        Ok(storage)
    }

    /// Create and bootstrap storage in the default agentkernel data directory.
    pub fn default() -> Result<Self> {
        Self::new(Self::default_db_path())
    }

    /// Resolve default SQLite database location.
    pub fn default_db_path() -> PathBuf {
        let base = dirs::home_dir()
            .map(|home| home.join(".local/share/agentkernel"))
            .unwrap_or_else(|| PathBuf::from("/tmp/agentkernel"));
        base.join("durable").join("orchestrations.db")
    }

    /// Return the storage path.
    #[allow(dead_code)]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Open a new SQLite connection.
    pub fn open_connection(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("failed to open sqlite db at {}", self.db_path.display()))?;
        Self::apply_pragmas(&conn)?;
        Ok(conn)
    }

    fn apply_pragmas(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA wal_autocheckpoint = 1000;
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = ON;
"#,
        )
        .context("failed to apply durable sqlite pragmas")
    }

    fn bootstrap(&self) -> Result<()> {
        if let Some(parent) = self.db_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create storage dir {}", parent.to_string_lossy())
            })?;
        }

        let mut conn = self.open_connection()?;
        conn.execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);
"#,
        )
        .context("failed to create sqlite migration metadata")?;

        let tx = conn
            .transaction()
            .context("failed to start migration transaction")?;
        for (version, sql) in MIGRATIONS {
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT version FROM schema_migrations WHERE version = ?1",
                    [*version],
                    |row| row.get(0),
                )
                .optional()
                .context("failed to read migration state")?;

            if existing.is_some() {
                continue;
            }

            tx.execute_batch(sql)
                .with_context(|| format!("failed applying migration {}", version))?;
            tx.execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES (?1, ?2)",
                params![*version, chrono::Utc::now().to_rfc3339()],
            )
            .with_context(|| format!("failed recording migration {}", version))?;
        }
        tx.commit().context("failed to commit migrations")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_bootstrap_creates_schema() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("durable.db");

        let storage = DurableStorage::new(db_path.clone()).unwrap();
        assert_eq!(storage.db_path(), db_path.as_path());
        assert!(db_path.exists());

        let conn = storage.open_connection().unwrap();
        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'orchestrations'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_exists, 1);

        let events_exists: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'events'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events_exists, 1);

        let migration_count: i64 = conn
            .query_row("SELECT COUNT(1) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(migration_count, 3);
    }
}
