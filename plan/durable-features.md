# Durable Sandboxes: Functions, Objects, and Stores

> RFC for agentkernel-d11 epic. Covers server-owned orchestration runtime,
> durable function orchestrations, durable object actors, and durable SQL
> stores — all built on agentkernel sandboxes.

## Problem

agentkernel sandboxes are ephemeral. If the server crashes mid-workflow, all
in-flight state is lost. Multi-step agent workflows (clone → build → test →
deploy) need crash-resilient orchestration. Long-lived stateful services
(caches, coordinators, counters) need a way to survive idle periods and
server restarts without manual lifecycle management.

## Design Principles

### 1. Server-Owned Runtime

The **server** drives execution. SDKs declare orchestrations and object
classes; the server invokes them. The orchestration function is a
**deterministic replay function** — it re-executes from the event log on
recovery, skipping completed activities.

The SDK never calls `POST /sandboxes` directly from inside an orchestration.
Instead, the orchestration function yields **activity descriptors** that the
server dispatches. This makes replay safe: the server knows which activities
already completed and returns cached results.

```
SDK (declares)  →  Server (drives)  →  Sandbox (executes)
                     ↕
               SQLite event log
```

### 2. Deterministic Replay

On restart or crash recovery, the server loads the event log from SQLite and
re-executes the orchestration function. The function sees cached results for
completed activities and only dispatches new work for pending ones.

**Replay rules:**
- Orchestration code must be deterministic: no `random()`, `Date.now()`, or
  I/O outside of activity calls.
- Timers and external events are replayed from the log, not re-waited.
- Non-deterministic operations (random IDs, timestamps) must be obtained via
  `ctx.newUuid()` or `ctx.currentTime()` which are logged and replayed.

### 3. Idempotency

Every activity execution is tagged with an **idempotency key** derived from
`(orchestration_id, activity_name, sequence_number)`. The server persists
this key with the result. On replay or retry, the server returns the cached
result instead of re-executing.

If the activity has external side effects (e.g., sending an email, calling
an API), the user is responsible for making those calls idempotent using the
provided idempotency key.

### 4. Retry and Failure Semantics

Activities support configurable retry policies:

```
RetryPolicy {
    max_attempts: u32,        // default: 3 (1 initial + 2 retries)
    initial_interval_ms: u64, // default: 1000
    backoff_coefficient: f64, // default: 2.0
    max_interval_ms: u64,     // default: 30000
    non_retryable_errors: Vec<String>,  // error types that skip retry
}
```

**Failure cascade:**
1. Activity fails → retry per policy (with exponential backoff).
2. All retries exhausted → `ActivityFailed` event written to log.
3. Orchestration receives the error — can catch and handle, or propagate.
4. Unhandled error → orchestration marked `Failed`.
5. Failed orchestrations can be **retried** (replays from the log, skipping
   completed activities) or **terminated** (marks as `Terminated`, releases
   resources).

**Timeouts:**
- `activity_timeout_ms`: Max wall-clock time for a single activity attempt.
- `orchestration_timeout_ms`: Max wall-clock time for the entire orchestration.
- Timeout → treated as a retryable failure.

### 5. SQLite Durability Model

Single-node durability via SQLite in WAL mode. No distributed consensus.

```
~/.local/share/agentkernel/durable/orchestrations.db
```

**WAL configuration:**
- `journal_mode = WAL` — concurrent reads during writes.
- `synchronous = NORMAL` — fsync on WAL commit (crash-safe for WAL).
- `wal_autocheckpoint = 1000` — checkpoint every 1000 pages (~4MB).
- `busy_timeout = 5000` — wait up to 5s for write lock.

**Durability guarantee:** A committed event survives process crash. It does
NOT survive disk failure. For production deployments on cloud, back up the
SQLite file or use the orchestration API's export endpoint.

### 6. Retention and Garbage Collection

Completed and terminated orchestrations/objects accumulate. Without GC, the
database grows unbounded.

**Default retention policy:**

| Status | Retention | Rationale |
|--------|-----------|-----------|
| Completed | 7 days | Allows post-hoc debugging |
| Failed | 30 days | Longer window for failure analysis |
| Terminated | 7 days | Same as completed |
| Running | Never GC'd | Active work |

**GC process:**
- Runs on daemon startup and every 6 hours.
- Deletes event log entries + instance metadata for expired orchestrations.
- `VACUUM` runs after GC if >20% of pages are free.
- Configurable via `[durable]` in `agentkernel.toml`:

```toml
[durable]
enabled = true
db_path = "~/.local/share/agentkernel/durable/orchestrations.db"

[durable.retention]
completed_days = 7
failed_days = 30
terminated_days = 7
gc_interval_hours = 6
vacuum_threshold_percent = 20
```

### 7. Durable Stores (SQLite + Postgres + MySQL + Redis)

Durable Stores expose SQL storage through a shared control-plane API while
preserving engine-specific behavior.

- Common lifecycle API (`create/list/get/delete`) across SDKs.
- Common execution API (`query` for reads, `execute` for writes).
- Engine-native SQL (no forced SQL dialect abstraction).
- Engine config remains explicit in `store.config`:
  - SQLite: file path + local durability settings.
  - Postgres: connection metadata / secret references.
  - MySQL: connection metadata / secret references.
  - Redis: host/db/credential metadata + command endpoint.

---

## Architecture

### Server-Side Components

```
src/
├── durable/
│   ├── mod.rs            # Module root, DurableEngine entry point
│   ├── store.rs          # SQLite event store (WAL mode)
│   ├── replay.rs         # Deterministic replay engine
│   ├── scheduler.rs      # Activity dispatch + retry logic
│   ├── gc.rs             # Retention enforcement + VACUUM
│   ├── orchestration.rs  # Orchestration instance management
│   └── object.rs         # Durable Object instance management
```

### SQLite Schema

See [durable-protocol.md](../docs/features/durable-protocol.md) for the
full schema. Summary:

- `orchestrations` — instance metadata (id, status, input, created_at).
- `events` — append-only event log per orchestration.
- `objects` — durable object instances (id, class, status, last_active).
- `object_storage` — key-value pairs per object (the "durable" state).
- `alarms` — scheduled object method calls.
- `stores` — durable store metadata (kind, config, sandbox, timestamps).

### HTTP API Extensions

```
POST   /orchestrations              # Start a new orchestration
GET    /orchestrations              # List orchestrations (with filters)
GET    /orchestrations/:id          # Get orchestration status + history
POST   /orchestrations/:id/events   # Send external event
POST   /orchestrations/:id/terminate
DELETE /orchestrations/:id          # Purge (removes all data)

POST   /objects/:class/:id/call     # Call a method on an object
GET    /objects                      # List objects (with filters)
GET    /objects/:class/:id           # Get object status + storage
DELETE /objects/:class/:id           # Delete object + storage

GET    /stores                       # List durable stores
POST   /stores                       # Create durable store
GET    /stores/:id                   # Get durable store metadata
DELETE /stores/:id                   # Delete durable store metadata
POST   /stores/:id/query             # Read query
POST   /stores/:id/execute           # Write statement
```

### SDK Wrappers

All 5 SDKs (Python, Node.js, Go, Rust, Swift) get thin wrappers:

- **Durable Functions**: `client.orchestration.start(name, input)`,
  `client.orchestration.status(id)`,
  `client.orchestration.signal(id, event)`,
  `client.orchestration.terminate(id)`.

- **Durable Objects**: `client.object(class, id).call(method, args)`,
  `client.object(class, id).status()`,
  `client.object(class, id).delete()`.

- **Durable Stores**: `client.stores.create(payload)`,
  `client.stores.query(id, sql, params)`,
  `client.stores.execute(id, sql, params)`.

SDKs are **thin HTTP clients**. All orchestration logic runs server-side.

---

## Phasing

### Phase 1: Event Store + Replay Engine (server-side)
- SQLite schema and store.rs
- Event log: append, query, replay cursor
- WAL mode configuration
- GC + retention

### Phase 2: Durable Functions (agentkernel-zai)
- Orchestration API endpoints
- Activity dispatch via sandbox exec
- Retry policy enforcement
- Deterministic replay on server restart
- SDK wrappers (all 5)

### Phase 3: Durable Objects (agentkernel-2sn)
- Object API endpoints
- In-sandbox HTTP server (port 9333) for method dispatch
- Hibernation: stop sandbox after idle timeout, persist storage
- Wake-on-call: auto-start sandbox on `call()`
- Alarms: scheduled method invocations via daemon
- SDK wrappers (all 5)

### Phase 4: Durable Stores (agentkernel-d11.1)
- Store API endpoints (`/stores`)
- SQLite execution path (`query` + `execute`)
- Postgres-compatible control-plane contract
- SDK wrappers (all 5)

### Phase 5: Cron Scheduling (agentkernel-0me)
- Daemon-integrated cron scheduler
- Cron expressions → orchestration triggers
- Desktop app UI (agentkernel-ov4)

---

## Comparison with Prior Art

| Feature | agentkernel | Azure Durable Functions | Temporal | Cloudflare DO |
|---------|-------------|------------------------|----------|---------------|
| Runtime | Server-owned replay | Server-owned replay | Server-owned replay | Worker-owned |
| Store | SQLite (single-node) | Azure Storage | Cassandra/SQL | Edge KV |
| Replay | Deterministic | Deterministic | Deterministic | N/A (no replay) |
| Scale | Single-node | Cloud-scale | Cloud-scale | Edge-scale |
| Isolation | microVM/container | Process | Process | V8 isolate |

agentkernel trades cloud-scale for simplicity: one SQLite database, one
server process, microVM-level isolation per activity.

---

## Non-Goals (for v1)

- Multi-node replication (use backup/restore instead).
- Versioning of orchestration code (user manages via code deployment).
- Built-in saga compensation (user implements via try/catch).
- Event sourcing for user-defined aggregates (this is infrastructure-only).

## Open Questions

1. Should the orchestration function run inside a sandbox itself, or purely
   server-side? **Decision: server-side.** The orchestration is a
   coordination function; only activities run in sandboxes.
2. Max event log size per orchestration before forcing `ContinueAsNew`?
   **Proposal: 10,000 events** (warn at 8,000).
3. Should Durable Objects support transactions across multiple objects?
   **Decision: no.** Single-object atomicity only, consistent with
   Cloudflare's model.
