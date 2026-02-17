# Durable Protocol

> Wire protocol for checkpoint, replay, and event logging used by Durable
> Functions and Durable Objects. This document defines the SQLite schema,
> event types, replay semantics, and API contracts.

## Overview

agentkernel's durable protocol is an **append-only event log** stored in
SQLite. The server writes events as orchestration activities execute. On
crash or restart, the server replays the log to reconstruct in-flight
orchestration state without re-executing completed activities.

The protocol is internal to the server. SDKs interact via the HTTP API; they
never read or write the event log directly.

Current implementation note: the executable server-side orchestration runtime
currently supports a narrow directive contract (`activity`, `wait_for_event`,
or no runtime directive). Richer orchestration definitions described in this
document are planned protocol surface and may not all be active in the current
runtime path.

## SQLite Schema

Database location: `~/.local/share/agentkernel/durable/orchestrations.db`

### Pragmas (applied on connection open)

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA wal_autocheckpoint = 1000;
PRAGMA busy_timeout = 5000;
PRAGMA foreign_keys = ON;
```

**Rationale:**
- `WAL` allows concurrent reads during writes (HTTP GET status while activity completes).
- `NORMAL` fsyncs the WAL on commit but not every write — safe against process crash, not disk loss.
- `busy_timeout = 5000` prevents immediate `SQLITE_BUSY` under concurrent access from the HTTP server and the scheduler.

### Tables

```sql
-- Orchestration instances
CREATE TABLE orchestrations (
    id           TEXT PRIMARY KEY,       -- UUIDv7
    name         TEXT NOT NULL,          -- orchestration type name
    status       TEXT NOT NULL DEFAULT 'Pending',
                 -- Pending | Running | Completed | Failed | Terminated | ContinuedAsNew
    input        BLOB,                   -- JSON-encoded input
    output       BLOB,                   -- JSON-encoded output (set on completion)
    error        TEXT,                    -- error message (set on failure)
    parent_id    TEXT REFERENCES orchestrations(id),  -- for sub-orchestrations
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now')),
    completed_at TEXT
);

CREATE INDEX idx_orchestrations_status ON orchestrations(status);
CREATE INDEX idx_orchestrations_name ON orchestrations(name);
CREATE INDEX idx_orchestrations_created ON orchestrations(created_at);

-- Append-only event log
CREATE TABLE events (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    orchestration_id   TEXT NOT NULL REFERENCES orchestrations(id) ON DELETE CASCADE,
    sequence           INTEGER NOT NULL,    -- per-orchestration sequence number
    event_type         TEXT NOT NULL,
    event_data         BLOB NOT NULL,       -- JSON-encoded event payload
    timestamp          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now')),
    UNIQUE(orchestration_id, sequence)
);

CREATE INDEX idx_events_orch ON events(orchestration_id, sequence);

-- Durable Object instances
CREATE TABLE objects (
    class        TEXT NOT NULL,
    id           TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'Active',
                 -- Active | Hibernating | Deleted
    sandbox_name TEXT,                    -- current sandbox (NULL when hibernating)
    sandbox_uuid TEXT,                    -- sandbox UUIDv7
    last_active  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now')),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now')),
    PRIMARY KEY (class, id)
);

-- Durable Object key-value storage
CREATE TABLE object_storage (
    class        TEXT NOT NULL,
    object_id    TEXT NOT NULL,
    key          TEXT NOT NULL,
    value        BLOB NOT NULL,           -- JSON-encoded
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now')),
    PRIMARY KEY (class, object_id, key),
    FOREIGN KEY (class, object_id) REFERENCES objects(class, id) ON DELETE CASCADE
);

-- Durable Object alarms
CREATE TABLE alarms (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    class        TEXT NOT NULL,
    object_id    TEXT NOT NULL,
    method       TEXT NOT NULL,
    args         BLOB,                    -- JSON-encoded
    fire_at      TEXT NOT NULL,            -- RFC3339 timestamp
    fired        INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (class, object_id) REFERENCES objects(class, id) ON DELETE CASCADE
);

CREATE INDEX idx_alarms_pending ON alarms(fire_at) WHERE fired = 0;

-- Durable Stores
CREATE TABLE stores (
    id           TEXT PRIMARY KEY,        -- UUIDv7
    name         TEXT NOT NULL UNIQUE,
    kind         TEXT NOT NULL,           -- sqlite | postgres | mysql | redis
    sandbox      TEXT,                    -- optional sandbox affinity
    config_json  BLOB NOT NULL,           -- JSON-encoded engine config
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now'))
);

CREATE INDEX idx_stores_kind ON stores(kind);
CREATE INDEX idx_stores_name ON stores(name);
```

## Event Types

Every event is a JSON object stored in `events.event_data`. The
`event_type` column contains the discriminator.

### Orchestration Lifecycle

| event_type | Payload | When |
|---|---|---|
| `OrchestratorStarted` | `{ "input": ... }` | Orchestration begins |
| `OrchestratorCompleted` | `{ "output": ... }` | Orchestration returns successfully |
| `OrchestratorFailed` | `{ "error": "...", "stack": "..." }` | Unhandled error |
| `OrchestratorTerminated` | `{ "reason": "..." }` | External termination |
| `ContinueAsNew` | `{ "input": ... }` | Orchestration resets with new input |

### Activity Events

| event_type | Payload | When |
|---|---|---|
| `ActivityScheduled` | `{ "name": "...", "input": ..., "idempotency_key": "...", "retry_policy": {...} }` | Orchestration yields an activity |
| `ActivityStarted` | `{ "sandbox_id": "...", "attempt": N }` | Server begins executing the activity |
| `ActivityCompleted` | `{ "output": ... }` | Activity returned successfully |
| `ActivityFailed` | `{ "error": "...", "attempt": N, "retryable": bool }` | Activity failed (may retry) |
| `ActivityTimedOut` | `{ "timeout_ms": N, "attempt": N }` | Activity exceeded timeout |

### Timer Events

| event_type | Payload | When |
|---|---|---|
| `TimerCreated` | `{ "fire_at": "...", "timer_id": "..." }` | Orchestration creates a timer |
| `TimerFired` | `{ "timer_id": "..." }` | Timer reached its fire time |

### External Events

| event_type | Payload | When |
|---|---|---|
| `EventRaised` | `{ "name": "...", "data": ... }` | External signal sent to orchestration |
| `EventConsumed` | `{ "name": "..." }` | Orchestration consumed the signal |

### Sub-Orchestration Events

| event_type | Payload | When |
|---|---|---|
| `SubOrchestrationCreated` | `{ "child_id": "...", "name": "...", "input": ... }` | Sub-orchestration spawned |
| `SubOrchestrationCompleted` | `{ "child_id": "...", "output": ... }` | Sub-orchestration finished |
| `SubOrchestrationFailed` | `{ "child_id": "...", "error": "..." }` | Sub-orchestration failed |

## Deterministic Replay

### How Replay Works

When the server restarts (or a new orchestration worker picks up an
instance), it replays the orchestration function:

1. Load all events for the orchestration, ordered by `sequence`.
2. Create a **replay context** that intercepts activity/timer calls.
3. Re-execute the orchestration function from the beginning.
4. For each `ctx.call_activity(name, input)`:
   - If a matching `ActivityCompleted` event exists at this sequence → return the cached output.
   - If a matching `ActivityFailed` event exists and all retries exhausted → raise the cached error.
   - If no matching event exists → this is **new work**; schedule the activity.
5. The replay function resumes until it either completes, fails, or yields new work.

### Replay Safety Rules

The orchestration function **must be deterministic**. Violations are
detected at replay time and cause a `NonDeterminismError`.

**Allowed inside orchestration functions:**
- `ctx.call_activity(name, input)` — dispatch work to a sandbox.
- `ctx.call_sub_orchestration(name, input)` — spawn child orchestration.
- `ctx.create_timer(duration)` — sleep for a duration.
- `ctx.wait_for_event(name)` — block until external signal.
- `ctx.current_time()` — returns the **replayed** timestamp (from event log).
- `ctx.new_uuid()` — returns a **replayed** UUID (logged on first execution).
- Pure computation, control flow, data transformations.

**Forbidden inside orchestration functions:**
- Direct I/O (network, filesystem, database queries).
- `Date.now()`, `Math.random()`, `uuid.v4()` (non-deterministic).
- Thread/goroutine spawning outside of the orchestration context.
- Global mutable state.

### Sequence Numbering

Each orchestration maintains a monotonically increasing sequence counter.
Every interaction with the replay context increments it:

```
Sequence 1: OrchestratorStarted
Sequence 2: ActivityScheduled("clone-repo")
Sequence 3: ActivityCompleted("clone-repo")     ← cached on replay
Sequence 4: ActivityScheduled("run-tests")
Sequence 5: ActivityCompleted("run-tests")       ← cached on replay
Sequence 6: OrchestratorCompleted
```

On replay, the server compares `(event_type, sequence)` pairs. If the
replayed function requests a different activity at a given sequence than
what's in the log, replay fails with `NonDeterminismError`.

## Idempotency Key Construction

```
idempotency_key = SHA256(orchestration_id || ":" || activity_name || ":" || sequence)
```

- The key is stored with the `ActivityScheduled` event.
- On retry, the same key is reused — the server checks for an existing
  `ActivityCompleted` with this key before executing.
- This means retries of the same activity at the same sequence are
  idempotent at the infrastructure level.
- If the activity itself makes external calls, users should forward the
  idempotency key to downstream services (available as
  `ctx.idempotency_key` in SDK callbacks).

## Retry Protocol

When an activity fails:

```
attempt 1: execute → fail
  wait initial_interval_ms (1000ms)
attempt 2: execute → fail
  wait initial_interval_ms * backoff_coefficient (2000ms)
attempt 3: execute → fail
  → all retries exhausted, write ActivityFailed event
```

Between retries:
- The server writes an `ActivityFailed` event with `retryable: true`.
- The wait is a server-side timer, not a sandbox operation.
- If the server crashes during a retry wait, it reconstructs the retry
  state from the event log on restart.

**Non-retryable errors:** If the error type matches `non_retryable_errors`,
the activity fails immediately without further retries.

## API Contracts

### POST /orchestrations

Start a new orchestration instance.

**Request:**
```json
{
  "name": "deploy-pipeline",
  "input": { "repo": "https://github.com/user/app", "ref": "main" },
  "retry_policy": {
    "max_attempts": 3,
    "initial_interval_ms": 1000,
    "backoff_coefficient": 2.0
  }
}
```

**Response (202 Accepted):**
```json
{
  "id": "019506e8-3b1f-7000-8000-000000000001",
  "name": "deploy-pipeline",
  "status": "Pending",
  "created_at": "2026-02-15T10:30:00.000Z"
}
```

The server returns 202, not 200 — the orchestration is accepted for
processing, not immediately complete.

### GET /orchestrations/:id

**Response (200):**
```json
{
  "id": "019506e8-3b1f-7000-8000-000000000001",
  "name": "deploy-pipeline",
  "status": "Running",
  "input": { "repo": "https://github.com/user/app", "ref": "main" },
  "output": null,
  "error": null,
  "created_at": "2026-02-15T10:30:00.000Z",
  "updated_at": "2026-02-15T10:30:05.000Z",
  "completed_at": null,
  "history": [
    { "sequence": 1, "type": "OrchestratorStarted", "timestamp": "..." },
    { "sequence": 2, "type": "ActivityScheduled", "data": { "name": "clone-repo" }, "timestamp": "..." },
    { "sequence": 3, "type": "ActivityCompleted", "data": { "output": "cloned" }, "timestamp": "..." }
  ]
}
```

### POST /orchestrations/:id/events

Send an external event (signal) to a running orchestration.

**Request:**
```json
{
  "name": "approval",
  "data": { "approved": true, "approver": "alice@example.com" }
}
```

**Response:** `202 Accepted`

### POST /orchestrations/:id/terminate

**Request:**
```json
{
  "reason": "Manual termination by operator"
}
```

**Response:** `200 OK`

Terminates the orchestration and stops any running activities. In-flight
sandbox operations are stopped (sandbox `stop()` called).

### POST /objects/:class/:id/call

Call a method on a Durable Object. Auto-creates the object if it doesn't
exist. Auto-wakes the sandbox if hibernating.

**Request:**
```json
{
  "method": "increment",
  "args": { "amount": 5 }
}
```

**Response (200):**
```json
{
  "result": { "value": 42 }
}
```

**Latency expectations:**
- Object active (sandbox running): <50ms (direct HTTP to in-sandbox server).
- Object hibernating: 1-5s (sandbox start + method dispatch).
- Object new: 1-5s (sandbox create + start + method dispatch).

### GET /objects/:class/:id

**Response (200):**
```json
{
  "class": "counter",
  "id": "user-123",
  "status": "Active",
  "sandbox_name": "do-counter-user-123",
  "sandbox_uuid": "019506e8-...",
  "last_active": "2026-02-15T10:30:00.000Z",
  "created_at": "2026-02-15T09:00:00.000Z",
  "storage": {
    "count": 42,
    "last_updated": "2026-02-15T10:30:00.000Z"
  }
}
```

### POST /stores

Create a durable store definition.

**Request:**
```json
{
  "name": "agent-state",
  "kind": "sqlite",
  "sandbox": "build-runner",
  "config": {
    "path": ".agentkernel/stores/agent-state.db"
  }
}
```

**Response (201):**
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

### POST /stores/:id/query

Read rows from a durable store.

**Request:**
```json
{
  "sql": "SELECT id, name FROM users WHERE id > ?",
  "params": [10]
}
```

**Response (200):**
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

### POST /stores/:id/execute

Execute a write statement.

**Request:**
```json
{
  "sql": "INSERT INTO users(name) VALUES (?)",
  "params": ["alice"]
}
```

**Response (200):**
```json
{
  "rows_affected": 1,
  "last_insert_rowid": 42
}
```

### POST /stores/:id/command

Execute command-oriented operations (Redis).

**Request:**
```json
{
  "command": ["SET", "session:123", "{\"ok\":true}"]
}
```

**Response (200):**
```json
{
  "result": "OK"
}
```

## Error Codes

| HTTP Status | Error Code | Meaning |
|---|---|---|
| 404 | `orchestration_not_found` | Orchestration ID does not exist |
| 404 | `store_not_found` | Store ID does not exist |
| 404 | `object_not_found` | Object class/id does not exist |
| 409 | `orchestration_already_completed` | Cannot signal/terminate a finished orchestration |
| 409 | `non_determinism_error` | Replay detected non-deterministic orchestration code |
| 422 | `invalid_orchestration_name` | Orchestration name not registered |
| 422 | `invalid_store_kind` | Store kind is not sqlite/postgres/mysql/redis |
| 422 | `invalid_store_command` | Invalid command payload for command endpoint |
| 422 | `invalid_method` | Object method not found |
| 503 | `sandbox_unavailable` | Cannot start sandbox for activity/object |
| 504 | `activity_timeout` | Activity exceeded configured timeout |

## Observability

Events in the log serve as the primary audit trail. Additionally:

- Prometheus metrics at `/metrics`:
  - `agentkernel_orchestrations_total{name, status}` — counter by final status.
  - `agentkernel_orchestration_duration_seconds{name}` — histogram.
  - `agentkernel_activities_total{name, status}` — counter by activity outcome.
  - `agentkernel_activity_duration_seconds{name}` — histogram.
  - `agentkernel_objects_active{class}` — gauge of active objects.
  - `agentkernel_replay_duration_seconds` — histogram of replay times.
  - `agentkernel_durable_db_size_bytes` — gauge.

- Audit log events (existing `AuditEvent` enum):
  - `OrchestrationStarted { id, name }`
  - `OrchestrationCompleted { id, name, duration_ms }`
  - `OrchestrationFailed { id, name, error }`
  - `ObjectCreated { class, id }`
  - `ObjectHibernated { class, id }`
  - `ObjectWoken { class, id }`
