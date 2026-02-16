# Durable Stores

> Durable storage bindings for SQLite, Postgres, MySQL, and Redis under a common
> control-plane API.

## Overview

Durable Stores provide a single API shape for SQL-backed state while keeping
engine-specific behavior explicit.

- `sqlite`: local file-backed database managed by the durable runtime.
- `postgres`: remote Postgres endpoint referenced by store config.
- `mysql`: remote MySQL endpoint referenced by store config.
- `redis`: command-oriented key/value store referenced by store config.

This abstraction is protocol-level (CRUD, query, execute, metadata), not a
SQL-dialect abstraction.

## Design Goals

- Common serialization contract across all SDKs.
- Stable store identity and lifecycle (`create/list/get/delete`).
- Explicit execution operations (`query` for reads, `execute` for writes).
- Engine-specific config remains visible in payloads.
- Idempotent control-plane behavior with deterministic API envelopes.

## API Surface

```text
GET    /stores
POST   /stores
GET    /stores/:id
DELETE /stores/:id

POST   /stores/:id/query
POST   /stores/:id/execute
POST   /stores/:id/command
```

## Store Model

```json
{
  "id": "019abc12-1234-7def-89ab-0123456789ab",
  "name": "agent-state",
  "kind": "sqlite",
  "sandbox": "build-runner",
  "config": {
    "path": ".agentkernel/stores/agent-state.db"
  },
  "created_at": "2026-02-16T00:00:00Z",
  "updated_at": "2026-02-16T00:00:00Z"
}
```

## Query/Execute Contract

Request:

```json
{
  "sql": "SELECT id, name FROM users WHERE id > ?",
  "params": [10]
}
```

Query response:

```json
{
  "columns": ["id", "name"],
  "rows": [
    {"id": 11, "name": "alice"},
    {"id": 12, "name": "bob"}
  ],
  "row_count": 2
}
```

Execute response:

```json
{
  "rows_affected": 1,
  "last_insert_rowid": 42
}
```

## Engine Notes

### SQLite

- Backed by a file on local durable storage.
- Suitable for local-first workflows and snapshots.
- Uses SQLite WAL mode defaults from the durable runtime.

### Postgres

- Store records are first-class and portable across SDKs.
- Execution contract matches SQLite (`query`/`execute`) with Postgres
  connection config in `store.config`.
- In-sandbox or remote execution policy can be enforced by runtime config.

### MySQL

- Uses the same SQL endpoint contract (`query`/`execute`).
- Keeps MySQL-native SQL semantics explicit.
- Config supports host/port/database/user and secret-backed credentials.

### Redis

- Uses command-oriented execution (`/stores/:id/command`).
- Keeps Redis-native semantics explicit (no SQL translation layer).
- Config supports host/port/db and secret-backed credentials.

## SDK Surface (All 5 SDKs)

- `listStores()`
- `createStore(payload)`
- `getStore(id)`
- `deleteStore(id)`
- `queryStore(id, payload)`
- `executeStore(id, payload)`
- `commandStore(id, payload)`

All SDKs keep payloads/rows as opaque JSON-compatible values to avoid
cross-language schema drift.

## Templates

Sandbox templates are included at:

- `templates/sqlite.toml`
- `templates/postgres.toml`
- `templates/mysql.toml`
- `templates/redis.toml`

API payload examples for `POST /stores` are included at:

- `examples/durable-stores/sqlite-store.json`
- `examples/durable-stores/postgres-store.json`
- `examples/durable-stores/mysql-store.json`
- `examples/durable-stores/redis-store.json`
