# Durable Functions

> SDK orchestrator with checkpoint-based replay for multi-step agent
> workflows. Tracks agentkernel-zai.

## What It Does

Durable Functions let you write multi-step workflows as ordinary code. The
server checkpoints after each step. If the server crashes, it replays the
function from the checkpoint log — completed steps return cached results,
only pending steps re-execute.

**Use case:** An AI agent pipeline that clones a repo, runs tests, and
deploys. Each step runs in an isolated sandbox. If the server restarts
after "clone" completes, it skips clone and resumes from "run tests."

## Architecture

```
SDK                           Server                        Sandbox
 |                              |                              |
 |-- start(name, input) ------->|                              |
 |                              |-- write OrchestratorStarted  |
 |                              |-- call orchestration fn      |
 |                              |   fn yields Activity("clone")|
 |                              |-- write ActivityScheduled    |
 |                              |-- create sandbox + exec ---->|
 |                              |                              |-- git clone
 |                              |<---- result -----------------|
 |                              |-- write ActivityCompleted    |
 |                              |   fn yields Activity("test") |
 |                              |-- write ActivityScheduled    |
 |                              |-- exec in sandbox ---------->|
 |                              |                              |-- cargo test
 |                              |<---- result -----------------|
 |                              |-- write ActivityCompleted    |
 |                              |-- write OrchestratorCompleted|
 |<-- 200 { status: Completed } |                              |
```

**Key point:** The orchestration function runs **server-side**, not in a
sandbox. Only activities (the actual work) run in sandboxes. This keeps the
orchestration lightweight and replayable.

## Current Executable Runtime Contract

This is the currently implemented server-side orchestration runtime contract.

- A server-side processing loop continuously polls orchestration instances in
  `Pending` and `Running` state and advances them.
- Runtime behavior is selected from the orchestration `input` payload.

### Supported Input Contracts (Current)

1. Activity directive:
```json
{
  "activity": {
    "name": "optional-name",
    "command": ["cmd", "arg1"],
    "image": "optional-image",
    "fast": true,
    "retry_policy": {
      "max_attempts": 3,
      "initial_interval_ms": 1000,
      "backoff_coefficient": 2.0,
      "max_interval_ms": 30000,
      "non_retryable_errors": ["PermissionDenied"]
    }
  }
}
```
`name`, `image`, and `fast` are optional. `command` is required.
`retry_policy` is optional; defaults are max 3 attempts with exponential
backoff (1s, 2s, 4s).

2. Wait-for-event directive:
```json
{
  "wait_for_event": "event-name"
}
```

3. No runtime directives:
- If neither `activity` nor `wait_for_event` is present, the orchestration
  completes and returns its input unchanged as output.

### Expected Event Transitions

Lifecycle labels below map to durable protocol events:
`Started`=`OrchestratorStarted`, `Scheduled`=`ActivityScheduled`,
`Completed`=`OrchestratorCompleted`/`ActivityCompleted`,
`Failed`=`OrchestratorFailed`/`ActivityFailed`,
`Terminated`=`OrchestratorTerminated`.

1. Activity path:
- `Started` -> `Scheduled` -> `Completed` (success)
- `Started` -> `Scheduled` -> `Failed` (activity attempt failure)
- `Started` -> `Scheduled` -> `Failed` ... -> `Completed` (after retries)
- `Terminated` can occur from `Pending` or `Running`

2. Wait-for-event path:
- `Started` -> `EventRaised` -> `EventConsumed` -> `Completed`
- `Terminated` can occur while waiting

3. No-directive path:
- `Started` -> `Completed`
- `Terminated` can occur before completion

## SDK API

### Python

```python
from agentkernel import AgentKernel

client = AgentKernel()

# Start an orchestration
result = client.orchestrations.start(
    name="deploy-pipeline",
    input={"repo": "https://github.com/user/app", "ref": "main"},
)
print(result.id)       # "019506e8-..."
print(result.status)   # "Pending"

# Poll for completion
status = client.orchestrations.get(result.id)
print(status.status)   # "Running" | "Completed" | "Failed"
print(status.output)   # available when Completed

# Send an external event (signal)
client.orchestrations.signal(result.id, name="approval", data={"ok": True})

# Terminate
client.orchestrations.terminate(result.id, reason="no longer needed")

# List orchestrations
items = client.orchestrations.list(status="Running", name="deploy-pipeline")
```

### Node.js / TypeScript

```typescript
import { AgentKernel } from "@anthropic/agentkernel";

const client = new AgentKernel();

const result = await client.orchestrations.start({
  name: "deploy-pipeline",
  input: { repo: "https://github.com/user/app", ref: "main" },
});

const status = await client.orchestrations.get(result.id);
await client.orchestrations.signal(result.id, {
  name: "approval",
  data: { ok: true },
});
await client.orchestrations.terminate(result.id, {
  reason: "no longer needed",
});
```

### Go

```go
client := agentkernel.New()

result, _ := client.Orchestrations.Start(ctx, agentkernel.StartOrchestration{
    Name:  "deploy-pipeline",
    Input: map[string]any{"repo": "https://github.com/user/app", "ref": "main"},
})

status, _ := client.Orchestrations.Get(ctx, result.ID)
```

### Rust

```rust
let client = AgentKernel::new();

let result = client.orchestrations().start("deploy-pipeline", &input).await?;
let status = client.orchestrations().get(&result.id).await?;
```

### Swift

```swift
let client = AgentKernel()

let result = try await client.orchestrations.start(
    name: "deploy-pipeline",
    input: ["repo": "https://github.com/user/app", "ref": "main"]
)
let status = try await client.orchestrations.get(id: result.id)
```

## Orchestration Patterns

### Sequential (Chain)

Activities execute one after another. Each receives the output of the
previous as input.

```
clone → build → test → deploy
```

Server-side orchestration definition:

```toml
# Registered via agentkernel.toml or API
[[orchestrations]]
name = "deploy-pipeline"
activities = [
    { name = "clone-repo", image = "alpine/git", command = ["git", "clone", "$input.repo"] },
    { name = "run-tests", image = "rust:1.82-alpine", command = ["cargo", "test"] },
    { name = "deploy",    image = "alpine:3.24",      command = ["sh", "deploy.sh"] },
]
```

### Fan-Out / Fan-In

Multiple activities execute in parallel. The orchestration waits for all to
complete before continuing.

```
        ┌─ lint ──────┐
clone ──┤              ├── deploy
        ├─ test ──────┤
        └─ typecheck ─┘
```

```toml
[[orchestrations]]
name = "parallel-checks"
steps = [
    { name = "clone", activity = "clone-repo" },
    { name = "checks", parallel = ["lint", "test", "typecheck"] },
    { name = "deploy", activity = "deploy", depends_on = "checks" },
]
```

### Human-in-the-Loop (External Events)

The orchestration pauses and waits for an external signal before continuing.

```
clone → test → [wait for approval] → deploy
```

The signal is sent via the API:
```
POST /orchestrations/:id/events
{ "name": "approval", "data": { "approved": true } }
```

### Sub-Orchestrations

An orchestration can spawn child orchestrations. The parent waits for the
child to complete. Sub-orchestration events are logged in the parent's event
log with `SubOrchestrationCreated` / `SubOrchestrationCompleted`.

### Continue-As-New

Long-running orchestrations can reset their event log to prevent unbounded
growth. The current instance completes with `ContinuedAsNew`, and a new
instance starts with fresh state but new input.

**When to use:** Polling loops, event processors, or any orchestration that
runs indefinitely. The server warns at 8,000 events and recommends
`ContinueAsNew` at 10,000.

## Retry Configuration

Activities inherit a default retry policy that can be overridden:

```json
{
  "max_attempts": 3,
  "initial_interval_ms": 1000,
  "backoff_coefficient": 2.0,
  "max_interval_ms": 30000,
  "non_retryable_errors": ["InvalidInput", "PermissionDenied"]
}
```

### Retry Behavior

| Attempt | Wait Before | Total Elapsed |
|---|---|---|
| 1 (initial) | 0 | 0s |
| 2 (1st retry) | 1s | 1s |
| 3 (2nd retry) | 2s | 3s |
| (exhausted) | — | `ActivityFailed` event |

**Timeout handling:** If an activity exceeds `activity_timeout_ms`, the
server kills the sandbox operation and writes `ActivityTimedOut`. This
counts as a retryable failure unless the timeout itself is in
`non_retryable_errors`.

**Crash during retry wait:** If the server crashes while waiting to retry,
the retry timer state is reconstructed from the event log on restart. The
`ActivityFailed` event with `retryable: true` records when the failure
occurred; the server calculates the remaining wait time.

## Server-Side Registration

Orchestrations must be registered before they can be started. Registration
tells the server what activities exist and how to execute them.

### Via agentkernel.toml

```toml
[[orchestrations]]
name = "deploy-pipeline"

[[orchestrations.activities]]
name = "clone-repo"
image = "alpine/git"
command = ["git", "clone", "--depth=1", "$input.repo", "/workspace"]
timeout_ms = 60000

[[orchestrations.activities]]
name = "run-tests"
image = "rust:1.82-alpine"
command = ["cargo", "test"]
timeout_ms = 300000
retry_policy = { max_attempts = 2 }

[[orchestrations.activities]]
name = "deploy"
image = "alpine:3.24"
command = ["sh", "/workspace/deploy.sh"]
timeout_ms = 120000
```

### Via API

```
POST /orchestrations/definitions
{
  "name": "deploy-pipeline",
  "activities": [...]
}
```

## Idempotency

Each activity execution carries an idempotency key:

```
SHA256(orchestration_id + ":" + activity_name + ":" + sequence_number)
```

**What this guarantees:**
- If the server crashes after completing an activity but before writing
  `ActivityCompleted`, the replay will re-execute the activity. The same
  idempotency key is generated, so downstream services that respect
  idempotency keys will not duplicate work.
- The SDK exposes the key as `ctx.idempotency_key` for activities that need
  to pass it to external services.

**What this does NOT guarantee:**
- Activities with non-idempotent side effects (e.g., `rm -rf`, sending a
  message) may execute twice if the server crashes between execution and
  checkpoint. Users must handle this at the application level.

## Observability

- `GET /orchestrations/:id` includes full event history.
- Prometheus metrics: `agentkernel_orchestrations_total`,
  `agentkernel_orchestration_duration_seconds`,
  `agentkernel_activities_total`, `agentkernel_activity_duration_seconds`.
- Audit log: `OrchestrationStarted`, `OrchestrationCompleted`,
  `OrchestrationFailed`.

## Limits

| Limit | Value | Rationale |
|---|---|---|
| Max event log per orchestration | 10,000 events | Prevents unbounded replay time |
| Max concurrent orchestrations | 1,000 | Single-node SQLite throughput |
| Max activity payload (input/output) | 1 MB | Stored in SQLite BLOB |
| Max orchestration input | 1 MB | Same |
| Max parallel activities per fan-out | 50 | Sandbox resource constraints |
| Max sub-orchestration depth | 10 | Prevent runaway nesting |
