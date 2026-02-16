# Durable Objects

> Stateful sandbox actors with hibernation and alarms. Tracks
> agentkernel-2sn.

## What It Does

A Durable Object is a sandbox that maintains persistent state across calls.
Each object has a unique identity (`class` + `id`), an in-sandbox HTTP
server for method dispatch, and key-value storage that survives hibernation
and restarts.

**Use case:** A per-user session cache, a rate limiter, a coordinator that
tracks agent progress across multiple sandboxes, or a build cache that
persists between CI runs.

## Architecture

```
SDK                          Server                        Sandbox (port 9333)
 |                             |                              |
 |-- call("counter","a",inc)-> |                              |
 |                             |-- lookup (class=counter,id=a)|
 |                             |   status = Hibernating       |
 |                             |-- start sandbox ------------>|
 |                             |-- restore storage to sandbox |
 |                             |-- POST :9333/increment ----->|
 |                             |                              |-- update state
 |                             |                              |-- return result
 |                             |<---- { "value": 42 } --------|
 |<-- { "result": {"value":42}}|                              |
 |                             |                              |
 |   (idle timeout elapses)    |                              |
 |                             |-- persist storage            |
 |                             |-- stop sandbox (hibernate)-->|
 |                             |   status = Hibernating       |
```

**Server-owned lifecycle:** The server decides when to start, hibernate,
and wake objects. The SDK is a thin HTTP client. The in-sandbox HTTP server
handles method dispatch — the server forwards calls to it.

## Object Lifecycle

```
                          ┌──────────────┐
           call()         │              │     idle timeout
  ┌────────────────────> Active  ─────────────────────────┐
  │                       │              │                │
  │                       └──────┬───────┘                v
  │                              │              ┌─────────────────┐
  │                          call()             │                 │
  │                              │              │  Hibernating    │
  │                              v              │                 │
  │                       ┌──────────────┐      └────────┬────────┘
  │                       │              │               │
  │                       │   Active     │<──────────────┘
  │                       │              │     call() (auto-wake)
  │                       └──────────────┘
  │
  │   delete()            ┌──────────────┐
  └──────────────────────>│   Deleted    │
                          └──────────────┘
```

### States

| State | Sandbox | Storage | Description |
|---|---|---|---|
| Active | Running | In-memory + persisted | Object is handling calls |
| Hibernating | Stopped | Persisted in SQLite | Object is idle, no sandbox running |
| Deleted | Stopped | Purged | Object and all storage removed |

### Hibernation

When an object has been idle for `idle_timeout` (default: 5 minutes), the
server:

1. Reads the object's storage from the in-sandbox HTTP server (`GET :9333/__storage`).
2. Persists the storage to SQLite (`object_storage` table).
3. Stops the sandbox.
4. Sets status to `Hibernating`.

On the next `call()`, the server:

1. Creates and starts a new sandbox.
2. Injects the stored key-value pairs via `POST :9333/__storage`.
3. Forwards the method call.
4. Sets status to `Active`.

**Consistency guarantee:** Storage is persisted atomically within a single
SQLite transaction. A crash during hibernation leaves the object in `Active`
state; the server re-reads storage on next hibernation attempt.

## In-Sandbox HTTP Server

Each Durable Object sandbox runs an HTTP server on port 9333. The server
handles method dispatch and storage management.

### Protocol

| Endpoint | Method | Purpose |
|---|---|---|
| `POST /:method` | POST | Call a method on the object |
| `GET /__storage` | GET | Dump all key-value pairs (for hibernation) |
| `POST /__storage` | POST | Restore key-value pairs (on wake) |
| `GET /__health` | GET | Health check |

### Method Dispatch

```
POST /increment
Content-Type: application/json

{ "amount": 5 }
```

Response:
```json
{ "value": 42 }
```

The in-sandbox server is user-defined code. The server provides a template
or SDK helper for each language to handle the boilerplate.

### Storage Protocol

**Dump (GET /__storage):**
```json
{
  "count": 42,
  "last_updated": "2026-02-15T10:30:00.000Z"
}
```

**Restore (POST /__storage):**
```json
{
  "count": 42,
  "last_updated": "2026-02-15T10:30:00.000Z"
}
```

Storage values are JSON. Max size per key: 1 MB. Max keys per object: 10,000.

## SDK API

### Python

```python
from agentkernel import AgentKernel

client = AgentKernel()

# Call a method (auto-creates + auto-wakes)
result = client.objects.call("counter", "user-123", method="increment", args={"amount": 5})
print(result)  # {"value": 42}

# Get object status
info = client.objects.get("counter", "user-123")
print(info.status)       # "Active" | "Hibernating"
print(info.storage)      # {"count": 42, ...}

# List objects
items = client.objects.list(class_name="counter", status="Active")

# Delete object + storage
client.objects.delete("counter", "user-123")

# Set an alarm
client.objects.set_alarm(
    "counter", "user-123",
    method="reset",
    fire_at="2026-02-16T00:00:00Z",
)
```

### Node.js / TypeScript

```typescript
import { AgentKernel } from "@anthropic/agentkernel";

const client = new AgentKernel();

const result = await client.objects.call("counter", "user-123", {
  method: "increment",
  args: { amount: 5 },
});

const info = await client.objects.get("counter", "user-123");
await client.objects.delete("counter", "user-123");

await client.objects.setAlarm("counter", "user-123", {
  method: "reset",
  fireAt: "2026-02-16T00:00:00Z",
});
```

### Go

```go
client := agentkernel.New()

result, _ := client.Objects.Call(ctx, "counter", "user-123", agentkernel.ObjectCall{
    Method: "increment",
    Args:   map[string]any{"amount": 5},
})

info, _ := client.Objects.Get(ctx, "counter", "user-123")
```

### Rust

```rust
let client = AgentKernel::new();

let result = client.objects().call("counter", "user-123", "increment", &args).await?;
let info = client.objects().get("counter", "user-123").await?;
```

### Swift

```swift
let client = AgentKernel()

let result = try await client.objects.call(
    class: "counter", id: "user-123",
    method: "increment", args: ["amount": 5]
)
let info = try await client.objects.get(class: "counter", id: "user-123")
```

## Alarms

Alarms schedule a method call on an object at a future time. The daemon's
cron scheduler fires alarms by calling the object's method via the same
`call()` path.

### Setting Alarms

```
POST /objects/counter/user-123/alarms
{
  "method": "reset",
  "args": {},
  "fire_at": "2026-02-16T00:00:00Z"
}
```

### Alarm Guarantees

- **At-least-once delivery:** If the server crashes after an alarm fires
  but before marking it `fired = 1`, it will fire again on restart.
- **No exact-time guarantee:** Alarms fire within 1 minute of `fire_at`.
  The daemon polls for pending alarms every 30 seconds.
- **Alarm deduplication:** Setting a new alarm for the same
  `(class, id, method)` replaces the previous pending alarm.

### Alarm Retry

If the method call fails, the alarm is retried with exponential backoff
(1s, 2s, 4s) up to 3 attempts. After 3 failures, the alarm is marked
`fired = 1` and an `AlarmFailed` audit event is logged.

## Object Registration

Objects must declare their class and supported methods before use.

### Via agentkernel.toml

```toml
[[objects]]
class = "counter"
image = "node:22-alpine"
idle_timeout_seconds = 300     # 5 minutes
init_command = ["node", "/app/counter-server.js"]

[[objects]]
class = "session-cache"
image = "python:3.12-alpine"
idle_timeout_seconds = 600     # 10 minutes
init_command = ["python", "/app/cache_server.py"]
```

### Via API

```
POST /objects/definitions
{
  "class": "counter",
  "image": "node:22-alpine",
  "idle_timeout_seconds": 300,
  "init_command": ["node", "/app/counter-server.js"]
}
```

## Naming and Addressing

Objects are addressed by `(class, id)`. The server maps this to a sandbox:

```
sandbox_name = "do-{class}-{id}"
```

Examples:
- `("counter", "user-123")` → sandbox `do-counter-user-123`
- `("session", "abc")` → sandbox `do-session-abc`

The sandbox name must be unique. If a sandbox with the same name already
exists (from a previous non-hibernated run), the server reuses it.

## Retry and Failure

### Method Call Failures

If a method call to the in-sandbox server fails:

1. **Sandbox not running** → auto-wake (start + restore storage + retry).
2. **HTTP error from in-sandbox server** → return error to caller (no retry).
3. **Sandbox start failure** → return 503 to caller with `sandbox_unavailable`.
4. **Timeout** (method takes >30s) → return 504 to caller.

Method calls are **not retried** by the server. The caller decides whether
to retry. This differs from Durable Functions activities, which have
server-side retry policies. Rationale: object methods may have
non-idempotent side effects (e.g., incrementing a counter), so blind retry
would cause double-counting.

### Storage Consistency

- Storage is eventually consistent with the in-sandbox state. The server
  reads storage from the sandbox on hibernation, not on every call.
- If the sandbox crashes without hibernating, the last persisted storage
  is used. Any in-memory-only changes since the last hibernation are lost.
- To force a storage checkpoint without hibernating, call
  `POST /__storage/checkpoint` on the in-sandbox server (the server will
  read and persist).

## Observability

- `GET /objects/:class/:id` includes storage dump and status.
- Prometheus metrics: `agentkernel_objects_active{class}`,
  `agentkernel_object_calls_total{class, method}`,
  `agentkernel_object_hibernations_total{class}`,
  `agentkernel_object_wake_duration_seconds{class}`.
- Audit log: `ObjectCreated`, `ObjectHibernated`, `ObjectWoken`,
  `ObjectDeleted`.

## Limits

| Limit | Value | Rationale |
|---|---|---|
| Max storage keys per object | 10,000 | SQLite row count |
| Max storage value size | 1 MB | SQLite BLOB |
| Max total storage per object | 50 MB | Disk budget |
| Max concurrent active objects | 200 | Sandbox resource constraints |
| Max alarms per object | 100 | Prevent alarm spam |
| Method call timeout | 30s | Prevent hung objects |
| Idle timeout | 5 min (default) | Configurable per class |
| Hibernation storage dump timeout | 10s | Prevent hung dumps |
