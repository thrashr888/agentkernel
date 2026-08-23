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
    (
        4,
        r#"
CREATE TABLE IF NOT EXISTS stores (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    sandbox TEXT,
    config_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_stores_name ON stores(name);
CREATE INDEX IF NOT EXISTS idx_stores_kind ON stores(kind);
"#,
    ),
    (
        5,
        r#"
CREATE TABLE IF NOT EXISTS objects (
    id TEXT PRIMARY KEY,
    class TEXT NOT NULL,
    object_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'hibernating',
    sandbox TEXT,
    storage_json TEXT NOT NULL DEFAULT '{}',
    idle_timeout_seconds INTEGER NOT NULL DEFAULT 300,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(class, object_id)
);

CREATE INDEX IF NOT EXISTS idx_objects_class ON objects(class);
CREATE INDEX IF NOT EXISTS idx_objects_status ON objects(status);
CREATE INDEX IF NOT EXISTS idx_objects_class_object_id ON objects(class, object_id);
"#,
    ),
    (
        6,
        r#"
CREATE TABLE IF NOT EXISTS schedules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    cron TEXT,
    fire_at TEXT,
    method TEXT NOT NULL,
    args_json TEXT NOT NULL DEFAULT '{}',
    target_class TEXT,
    target_object_id TEXT,
    target_orchestration TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    last_fired_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_schedules_status ON schedules(status);
CREATE INDEX IF NOT EXISTS idx_schedules_fire_at ON schedules(fire_at);
"#,
    ),
    (
        7,
        r#"
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    prompt TEXT NOT NULL,
    sandbox TEXT NOT NULL,
    status TEXT NOT NULL,
    result TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(created_at DESC);
"#,
    ),
    (
        8,
        r#"
ALTER TABLE tasks ADD COLUMN isolation_json TEXT;
"#,
    ),
    (
        9,
        r#"
ALTER TABLE tasks ADD COLUMN worker_id TEXT;
ALTER TABLE tasks ADD COLUMN lease_expires_at TEXT;
CREATE INDEX IF NOT EXISTS idx_tasks_lease ON tasks(status, lease_expires_at);
"#,
    ),
    (
        10,
        r#"
CREATE TABLE IF NOT EXISTS scheduled_job_runs (
    schedule_id TEXT PRIMARY KEY,
    last_run_at TEXT,
    last_run_minute INTEGER,
    last_status TEXT,
    last_error TEXT,
    next_run_at TEXT,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_scheduled_job_runs_next
    ON scheduled_job_runs(next_run_at);
CREATE TABLE IF NOT EXISTS scim_users (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    external_id TEXT,
    user_name TEXT NOT NULL COLLATE NOCASE,
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    display_name TEXT,
    given_name TEXT,
    family_name TEXT,
    email TEXT,
    locale TEXT,
    timezone TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_scim_users_tenant ON scim_users(tenant_id);
CREATE INDEX IF NOT EXISTS idx_scim_users_tenant_external_id
    ON scim_users(tenant_id, external_id);
CREATE UNIQUE INDEX IF NOT EXISTS uq_scim_users_tenant_user_name
    ON scim_users(tenant_id, user_name COLLATE NOCASE) WHERE deleted = 0;
CREATE UNIQUE INDEX IF NOT EXISTS uq_scim_users_tenant_external_id
    ON scim_users(tenant_id, external_id) WHERE deleted = 0 AND external_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS scim_groups (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    external_id TEXT,
    display_name TEXT NOT NULL COLLATE NOCASE,
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_scim_groups_tenant ON scim_groups(tenant_id);
CREATE UNIQUE INDEX IF NOT EXISTS uq_scim_groups_tenant_display_name
    ON scim_groups(tenant_id, display_name COLLATE NOCASE) WHERE active = 1;
CREATE UNIQUE INDEX IF NOT EXISTS uq_scim_groups_tenant_external_id
    ON scim_groups(tenant_id, external_id) WHERE active = 1 AND external_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS scim_group_members (
    group_id TEXT NOT NULL REFERENCES scim_groups(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES scim_users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (group_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_scim_group_members_user ON scim_group_members(user_id);
"#,
    ),
    (
        11,
        r#"
CREATE TABLE IF NOT EXISTS scim_principal_bindings (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES scim_users(id) ON DELETE CASCADE,
    group_id TEXT NOT NULL REFERENCES scim_groups(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT '',
    team_id TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, user_id, group_id, role, team_id)
);

CREATE INDEX IF NOT EXISTS idx_scim_principal_bindings_user
    ON scim_principal_bindings(tenant_id, user_id);
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
    pub fn open_default() -> Result<Self> {
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
        assert_eq!(migration_count, 10);
    }

    #[test]
    fn test_bootstrap_migrates_existing_database_without_losing_data() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("legacy.db");

        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);
CREATE TABLE orchestrations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    input_json TEXT,
    output_json TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
INSERT INTO schema_migrations(version, applied_at) VALUES (1, '2025-01-01T00:00:00Z');
INSERT INTO orchestrations(
    id, name, status, input_json, output_json, error, created_at, updated_at
) VALUES (
    'legacy-id', 'legacy-run', 'completed', '{"source":"existing"}',
    '{"preserved":true}', NULL, '2025-01-01T00:00:00Z', '2025-01-01T00:01:00Z'
);
"#,
        )
        .unwrap();
        drop(conn);

        let storage = DurableStorage::new(db_path).unwrap();
        let conn = storage.open_connection().unwrap();

        let preserved: (String, String, String) = conn
            .query_row(
                "SELECT name, input_json, output_json FROM orchestrations WHERE id = 'legacy-id'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(preserved.0, "legacy-run");
        assert_eq!(preserved.1, r#"{"source":"existing"}"#);
        assert_eq!(preserved.2, r#"{"preserved":true}"#);

        let versions: Vec<i64> = conn
            .prepare("SELECT version FROM schema_migrations ORDER BY version")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(versions, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }
}
