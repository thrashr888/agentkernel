//! Service-backed durable-store compatibility checks.
//!
//! These tests are ignored locally and run in the dedicated store workflow.

use agentkernel::durable_storage::DurableStorage;
use agentkernel::orchestration_store::{CreateDurableStore, DurableStoreKind, OrchestrationStore};

fn test_store() -> (tempfile::TempDir, OrchestrationStore) {
    let temp = tempfile::TempDir::new().expect("create temporary durable-store directory");
    let path = temp.path().join("control-plane.db");
    let store = OrchestrationStore::new(
        DurableStorage::new(path).expect("create durable-store metadata db"),
    );
    (temp, store)
}

fn env_port(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn redis_round_trip(name: &str, port: u16) {
    let (_temp, store) = test_store();
    let record = store
        .create_store(CreateDurableStore {
            name: name.to_string(),
            kind: DurableStoreKind::Redis,
            sandbox: None,
            config: Some(serde_json::json!({
                "host": "127.0.0.1",
                "port": port,
                "db": 0
            })),
        })
        .expect("create Redis-compatible store");

    let set = store
        .command_store(
            &record.id,
            vec![
                "SET".into(),
                "agentkernel:compat".into(),
                "round-trip".into(),
            ],
        )
        .expect("execute SET")
        .expect("store exists");
    assert_eq!(set.result, serde_json::json!("OK"));

    let get = store
        .command_store(&record.id, vec!["GET".into(), "agentkernel:compat".into()])
        .expect("execute GET")
        .expect("store exists");
    assert_eq!(get.result, serde_json::json!("round-trip"));

    store
        .command_store(&record.id, vec!["DEL".into(), "agentkernel:compat".into()])
        .expect("execute DEL");
}

#[test]
#[ignore = "requires the Redis compatibility service"]
fn redis_7_round_trip() {
    redis_round_trip("redis-7", env_port("REDIS_PORT", 6379));
}

#[test]
#[ignore = "requires the Valkey compatibility service"]
fn valkey_9_round_trip() {
    redis_round_trip("valkey-9", env_port("VALKEY_PORT", 6380));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the PostgreSQL compatibility service"]
async fn postgres_17_round_trip() {
    let (_temp, store) = test_store();
    let record = store
        .create_store(CreateDurableStore {
            name: "postgres-17".into(),
            kind: DurableStoreKind::Postgres,
            sandbox: None,
            config: Some(serde_json::json!({
                "host": "127.0.0.1",
                "port": env_port("POSTGRES_PORT", 5432),
                "user": "agentkernel",
                "password": "agentkernel",
                "dbname": "agentkernel"
            })),
        })
        .expect("create PostgreSQL store");

    store
        .execute_store(
            &record.id,
            "CREATE TABLE IF NOT EXISTS agentkernel_store_compat (id INTEGER PRIMARY KEY, value TEXT NOT NULL)",
            vec![],
        )
        .expect("create PostgreSQL table");
    store
        .execute_store(&record.id, "DELETE FROM agentkernel_store_compat", vec![])
        .expect("clear PostgreSQL table");
    store
        .execute_store(
            &record.id,
            "INSERT INTO agentkernel_store_compat(id, value) VALUES (1, 'round-trip')",
            vec![],
        )
        .expect("insert PostgreSQL row");

    let queried = store
        .query_store(
            &record.id,
            "SELECT id, value FROM agentkernel_store_compat WHERE id = 1",
            vec![],
        )
        .expect("query PostgreSQL row")
        .expect("store exists");
    assert_eq!(queried.row_count, 1);
    assert_eq!(queried.rows[0]["id"], serde_json::json!(1));
    assert_eq!(queried.rows[0]["value"], serde_json::json!("round-trip"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the MySQL compatibility service"]
async fn mysql_8_4_round_trip() {
    let (_temp, store) = test_store();
    let record = store
        .create_store(CreateDurableStore {
            name: "mysql-8-4".into(),
            kind: DurableStoreKind::Mysql,
            sandbox: None,
            config: Some(serde_json::json!({
                "host": "127.0.0.1",
                "port": env_port("MYSQL_PORT", 3306),
                "user": "root",
                "password": "agentkernel",
                "dbname": "agentkernel"
            })),
        })
        .expect("create MySQL store");

    store
        .execute_store(
            &record.id,
            "CREATE TABLE IF NOT EXISTS agentkernel_store_compat (id INTEGER PRIMARY KEY, value VARCHAR(64) NOT NULL)",
            vec![],
        )
        .expect("create MySQL table");
    store
        .execute_store(&record.id, "DELETE FROM agentkernel_store_compat", vec![])
        .expect("clear MySQL table");
    store
        .execute_store(
            &record.id,
            "INSERT INTO agentkernel_store_compat(id, value) VALUES (1, 'round-trip')",
            vec![],
        )
        .expect("insert MySQL row");

    let queried = store
        .query_store(
            &record.id,
            "SELECT id, value FROM agentkernel_store_compat WHERE id = 1",
            vec![],
        )
        .expect("query MySQL row")
        .expect("store exists");
    assert_eq!(queried.row_count, 1);
    // mysql_async's text protocol returns integer columns as byte strings.
    assert_eq!(queried.rows[0]["id"], serde_json::json!("1"));
    assert_eq!(queried.rows[0]["value"], serde_json::json!("round-trip"));
}
