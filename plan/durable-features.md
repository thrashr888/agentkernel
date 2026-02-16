# Durable Workflows and Actors

## Problem

AI agents need long-running, stateful workflows. Today, agentkernel sandboxes are ephemeral (create, exec, destroy). There is no built-in way to:

1. Chain sandbox executions with automatic state persistence between steps
2. Run stateful services that hibernate when idle and wake on demand
3. Schedule recurring work (cron-like) on sandbox-backed functions
4. Fan out work to parallel sandboxes and aggregate results safely

Azure Durable Functions and Cloudflare Durable Objects solve these problems in their own runtimes. agentkernel can provide equivalent patterns with VM-level isolation.

## Design Principles

1. **Server-owned execution**: durable runtime lives in agentkernel server/daemon, not user SDK process
2. **Deterministic replay**: replay from event history, never serialize language runtime closure state
3. **Protocol first**: one durable schema + wire protocol across all SDKs
4. **Explicit guarantees**: document delivery, retry, locking, idempotency, and durability modes
5. **Safety by default**: tenancy, authz, audit logs, quotas, and backpressure are part of MVP

## Feature 0: Durable Protocol + Canonical Schema

### Why

Without a common schema and protocol, each SDK invents slightly different payload shapes, replay behavior, and serialization edge cases. That breaks interoperability and cross-language tooling.

### Scope

Define one protocol contract used by HTTP API, daemon storage, and all SDKs:

- Canonical IDs: `instance_id`, `object_id`, `activity_id`, `event_id` (UUIDv7)
- Canonical envelopes:
  - `OrchestrationStartRequest`
  - `OrchestrationEvent`
  - `ActivityScheduled` / `ActivityCompleted` / `ActivityFailed`
  - `TimerCreated` / `TimerFired`
  - `ExternalEventRaised`
  - `ObjectCallRequest` / `ObjectCallResult`
- Canonical result payload:
  - `result` (structured JSON)
  - `logs` (stdout/stderr chunks, size-limited)
  - `artifacts` (optional references)
  - `metrics` (duration, exit code, retries)
- Error model:
  - typed error code
  - retryability flag
  - idempotency key
- Versioning:
  - protocol version field in every envelope
  - additive changes only within major version
  - explicit migration rules for persisted history

### Encoding choice

Use JSON as the canonical wire/storage representation first (debuggable, language-neutral), with optional protobuf mirrors later for high-volume links. The schema is the source of truth either way.

### Determinism and SDK wrappers

To keep replay deterministic, SDKs must provide deterministic context helpers for common non-deterministic operations, backed by protocol events:

- Time helper (for example `ctx.now()` / language-idiomatic equivalent)
- Random helper (deterministic seed or recorded values)
- UUID helper (deterministic derivation or recorded values)

Runtime rule: direct wall-clock/random APIs in orchestrator logic are unsupported for replay guarantees.

---

## Primitive Mapping

| Concept | Azure / Cloudflare | agentkernel |
|---------|--------------------|-------------|
| Execution unit | Function / Worker | Sandbox + exec |
| State persistence | Framework-managed | Event log + filesystem/snapshots |
| Isolation | Process / V8 isolate | Full VM (stronger) |
| Addressing | Instance ID / Object ID | Name + UUIDv7 |
| Concurrency control | Deterministic replay / actor queue | Server runtime + per-object call queue |
| Network | Cloud routing | Proxy + secret injection |
| Scheduled wake | Timers / Alarms | Daemon scheduler |
| Checkpointing | Event sourcing replay | Event-sourced history |
| Fan-out | Parallel functions | Parallel sandboxes with quotas |

---

## Feature 1: Sandbox UUIDs

### Problem

Sandboxes are currently addressed by user-chosen names. At scale (multi-tenant orchestration), names collide and are not globally unique.

### Design

Add a `uuid` field to `SandboxState`:

```rust
pub struct SandboxState {
    pub uuid: String, // UUIDv7
    pub name: String, // human-friendly alias
    // ... existing fields
}
```

### API changes

- Existing endpoints continue to accept `name` in paths
- Add `uuid` to all sandbox API responses
- New endpoint: `GET /sandboxes/by-uuid/{uuid}`
- `POST /sandboxes` returns `uuid`
- SDKs expose `sandbox.uuid`

### Migration

Backfill UUIDs for existing sandboxes on server load (`VmManager::new()`).

---

## Feature 2: Durable Workflows (Server Orchestrations)

### Concept

An orchestrator defines workflow logic. The **runtime is owned by agentkernel server/daemon**. SDKs are control-plane clients that submit definitions, start instances, and consume status/events.

### Architecture

```
SDK client                     agentkernel server/daemon
┌──────────────────┐          ┌──────────────────────────────┐
│ workflow DSL      │──HTTP──►│ /orchestrations              │
│ start/status/event│◄─HTTP───│ orchestrator runtime         │
└──────────────────┘          │ event store + scheduler       │
                              │ activity executor -> sandboxes│
                              └──────────────────────────────┘
```

### Execution semantics (MVP)

- Deterministic replay from durable event history
- At-least-once activity execution
- Activities must be idempotent (idempotency key provided in context)
- No serialization of async closures/stack frames
- Timers and external events are persisted as first-class events

### Patterns

1. Function chaining (`callActivity` sequence)
2. Fan-out/fan-in (`ctx.all([...])`) with per-tenant concurrency caps
3. Async HTTP (start + poll + fetch result)
4. Monitor loops (`createTimer` backoff)
5. Human interaction (`waitForExternalEvent` + timeout)
6. Sub-orchestrations (`callSubOrchestration`)

### Checkpointing

State is event-sourced server-side and persisted in SQLite (daemon-managed), not ad hoc log files:

```
~/.local/share/agentkernel/state.db
```

Core tables:

- `orchestrations`: instance metadata, status, inputs/outputs, timestamps
- `history`: ordered orchestration events keyed by `instance_id`
- `timers`: scheduled wakeups and timer state

Replay re-runs orchestrator logic against ordered history rows. Completed activities are not re-executed unless explicitly retried.

### Activity execution model

Each activity:
1. Acquires sandbox capacity (warm pool preferred)
2. Runs command
3. Produces structured result envelope (`result`, `logs`, `artifacts`, `metrics`)
4. Returns sandbox to pool or removes it
5. Appends completion/failure event to history

### Server-side API

```
POST   /orchestrations
GET    /orchestrations
GET    /orchestrations/{id}
POST   /orchestrations/{id}/events/{name}
POST   /orchestrations/{id}/retry/{activity-id}
DELETE /orchestrations/{id}
GET    /orchestrations/{id}/history
```

---

## Feature 3: Durable Actors (Stateful Sandboxes)

### Concept

A Durable Actor is a sandbox-backed actor with durable identity and serialized method execution.

- Globally unique identity (UUID + name)
- State persists across stop/start
- Auto-hibernates after idle timeout
- Auto-wakes on call/alarm
- Method calls are processed through a **single in-flight queue per object**

### Concurrency guarantees

- One active method call at a time per object
- Calls are queued FIFO by default
- Configurable call timeout and cancellation behavior
- At-least-once alarm delivery

### Durability modes

1. `ephemeral`: state lost on removal
2. `restart_persistent`: state survives stop/start in sandbox filesystem
3. `durable_snapshot`: snapshot persisted on hibernate/checkpoints

Expose durability mode in create/get APIs.

### Server-side API

```
POST   /objects
GET    /objects
GET    /objects/{id}
POST   /objects/{id}/call/{method}
GET    /objects/{id}/state
POST   /objects/{id}/hibernate
POST   /objects/{id}/wake
POST   /objects/{id}/alarms
GET    /objects/{id}/alarms
DELETE /objects/{id}/alarms/{alarm-id}
DELETE /objects/{id}
```

---

## Feature 4: Cron Scheduling (Daemon Integration)

### Concept

Extend daemon scheduler to:

1. Wake durable objects on alarm
2. Start orchestration instances on schedule
3. Run recurring sandbox commands

### Scheduling semantics (required)

- Explicit timezone per schedule (default from daemon config)
- DST behavior documented (`skip`, `run_once`, or `run_twice` policy)
- Missed-run policy after downtime (`catch_up` true/false)
- Retry policy (attempts, backoff)
- Idempotency key per scheduled invocation

### Configuration

```toml
[[schedule]]
name = "nightly-cleanup"
cron = "0 3 * * *"
timezone = "America/Los_Angeles"
target = { type = "sandbox", sandbox = "cleanup-worker", command = ["python", "/app/cleanup.py"] }
retry = { max_attempts = 3, backoff = "exponential" }
catch_up = false

[[schedule]]
name = "health-monitor"
cron = "*/5 * * * *"
timezone = "UTC"
target = { type = "orchestration", orchestration = "health-check-workflow" }
```

### Server-side API

```
POST   /schedules
GET    /schedules
GET    /schedules/{id}
PUT    /schedules/{id}
DELETE /schedules/{id}
GET    /schedules/{id}/runs
```

---

## Feature 5: Desktop App UI Affordances

### Workflow navigation

```
Workflow
├── Sandboxes
├── Templates
├── Snapshots
├── Secrets
├── Orchestrations  (new)
└── Objects         (new)
```

### Orchestration UI

- List: status, name, instance UUID, start time, duration, activity progress
- Detail: timeline, activity attempts, external events, retry controls
- Safety panels: tenant, auth policy, quota usage, idempotency keys

### Workflow debugger

- Replay inspector for completed workflow event history
- Step-through view by event index/timestamp
- State/output panel for each completed activity

### Objects UI

- List: status, name, UUID, last active, next alarm, state size
- Detail: state inspector, method log, alarm log, queue depth, durability mode
- Actions: wake, hibernate, delete, call method

### Schedules UI

Under System:

```
System
├── Audit Log
├── Diagnostics
├── Schedules  (new)
└── Settings
```

- Cron + timezone + next run
- Retry/catch-up configuration
- Run history with idempotency key and outcome

### Sandbox detail enhancements

- Show sandbox UUID
- Show orchestration linkage
- Show durable object badge and object ID

---

## Cross-Cutting Concerns

### Security and tenancy

- All durable APIs require authn/authz checks
- Tenant-scoped IDs and list endpoints
- Audit all mutating operations (start, event raise, retry, delete, alarm set)
- State redaction controls for UI and API

### Quotas and backpressure

- Per-tenant limits: active orchestrations, fan-out width, object count, schedule count
- Global worker capacity enforcement in daemon
- Queue depth metrics and rejection/error semantics when limits are hit

### Observability

- Metrics: activity duration, retries, queue depth, alarm lag, replay latency
- Structured logs with correlation IDs
- Trace propagation from SDK request to sandbox exec

### Retention and pruning

- Default TTL: completed/failed/terminated workflow instances retained for 7 days (configurable)
- Daily daemon pruning job removes expired orchestration rows, history rows, and timer rows
- Future: optional archival/export before pruning

---

## Implementation Phases

### Phase 0: Semantics + protocol RFC

**Effort**: Medium  
**Output**: durable protocol/schema spec + execution guarantees doc

1. Define wire/storage schemas and versioning
2. Define replay semantics and error taxonomy
3. Define durability modes and object concurrency guarantees
4. Define security + quota model

### Phase 1: Sandbox UUIDs

**Effort**: Small  
**Files**: `src/vmm.rs`, `src/http_api.rs`, SDK and app types

1. Add UUIDv7 to `SandboxState`
2. Backfill on load
3. Expose UUID in APIs and SDK/app types

### Phase 2: Durable Workflows MVP (server runtime + TypeScript SDK)

**Effort**: Medium  
**Files**: `src/http_api.rs`, runtime modules, `sdk/nodejs/*`

1. Server-owned orchestrator runtime + SQLite event store
2. Activity executor with structured result envelope
3. Timers and external events
4. Minimal TS SDK bindings

### Phase 3: Durable Actors MVP (server runtime + TypeScript SDK)

**Effort**: Medium  
**Files**: object runtime modules, `src/http_api.rs`, `sdk/nodejs/*`

1. In-sandbox server bootstrap
2. Per-object single-flight queue
3. Hibernation + alarms
4. Durability mode support

### Phase 4: Scheduler integration

**Effort**: Medium  
**Files**: `src/scheduler.rs`, daemon integration, config parsing

1. Cron parser + timezone support
2. Retry and catch-up semantics
3. Schedule run history and metrics

### Phase 5: Remaining SDKs

**Effort**: Medium  
**Files**: Python, Go, Rust, Swift SDKs

1. Generate/implement durable protocol models
2. Add functions/objects clients
3. Conformance tests against protocol fixtures

### Phase 6: Desktop App UI

**Effort**: Medium  
**Files**: Tauri commands, React pages, sidebar

1. Orchestrations and objects pages
2. Schedules page
3. Quota/security/observability panels

### Phase 7: Docs and examples

**Effort**: Small  
**Files**: docs/features, examples, API reference

1. Durable protocol doc
2. One end-to-end orchestrations example + one object example
3. Failure-mode and operations guide

---

## Relationship to Existing Beads

| Bead | Relationship |
|------|-------------|
| `agentkernel-d11` | Existing durable work tracker; this plan should map phases to this bead and its children instead of opening a parallel epic |
| `agentkernel-2w1.2` (Workspace scheduling) | Shares daemon scheduler; workspace scheduling is infra-level, durable schedules are user-level |
| `agentkernel-dse` (Agent Task Queue) | Durable Workflows is external orchestration model; queue can be one activity backend |
| `agentkernel-dse.3` (Parallel agent coordinator) | Fan-out/fan-in maps directly with quota controls |
| `agentkernel-hx7` (Desktop App features) | UI affordances for orchestrations, objects, and schedules |

---

## New Beads to Create

Use `agentkernel-d11` as the parent tracker. Create/update child beads under `agentkernel-d11` for:

1. Durable protocol + schema RFC (P1)
2. Sandbox UUIDs (P2)
3. Durable Workflows MVP (server runtime + TS SDK) (P2)
4. Durable Actors MVP (server runtime + TS SDK) (P2)
5. Cron scheduling with timezone/retry semantics (P2)
6. Multi-SDK rollout + conformance tests (P2)
7. Desktop app durable UX (P3)

---

## Open Questions

1. **Schema format**: JSON Schema only, or JSON Schema + protobuf generation? Recommendation: JSON Schema first, add protobuf once throughput needs it.
2. **Event retention**: Compaction/snapshot frequency for long histories. Recommendation: periodic snapshots + bounded hot history.
3. **Cross-region/multi-node runtime**: single daemon leader vs distributed coordination. Recommendation: single-leader first, pluggable coordinator later.
4. **Naming**: standardize user-facing names as "Durable Workflows" and "Durable Actors", with Azure/Cloudflare term mapping in docs.
