
# Durable Workflows (Durable Functions-style)

Chain sandbox executions into workflows with automatic checkpointing. If an SDK process or the server restarts, the workflow resumes from the durable event history and completed activities are not re-executed unless explicitly retried.

Inspired by [Azure Durable Functions](https://learn.microsoft.com/en-us/azure/azure-functions/durable/durable-functions-overview), built on agentkernel sandboxes for VM-level isolation.

## Why Durable Workflows?

AI agents run multi-step workflows: fetch data, process it, fan out to parallel workers, aggregate results. Without durability, a crash means starting over. Durable Workflows checkpoint after each step so you only re-run what's needed.

Each activity runs in its own sandbox — a full VM with a dedicated Linux kernel. Stronger isolation than any FaaS platform.

## Core Concepts

| Concept | Description |
|---------|-------------|
| **Orchestrator** | Workflow logic executed by the agentkernel durable runtime (server-side) |
| **Activity** | A single step that runs in a sandbox (create → exec → capture → remove) |
| **Checkpoint** | Event-sourced history append after each orchestration event |
| **Instance** | A running execution of an orchestration, identified by UUID |

## Durable Protocol (Cross-SDK)

Durable Workflows and Durable Actors share one protocol contract across SDKs and HTTP APIs:

- Canonical IDs (`instance_id`, `activity_id`, `event_id`)
- Canonical event envelopes (`activity_completed`, `timer_fired`, `event_received`, etc.)
- Canonical activity result envelope (`result`, `logs`, `artifacts`, `metrics`)
- Versioned schema for backward-compatible evolution

See [Durable Protocol](durable-protocol.md) for the envelope schema and versioning rules.

## Determinism Helpers

Replay correctness depends on deterministic orchestration logic. Use SDK context helpers (language-idiomatic names) for:

- current time (`ctx.now()`)
- random values (`ctx.random()`)
- UUID generation (`ctx.newUuid()`)

Direct use of wall-clock/random APIs inside orchestration logic is not replay-safe.

## Patterns

### Function Chaining

Sequential steps where each activity's output feeds the next.

### Fan-out / Fan-in

Run N activities in parallel (each in its own sandbox), wait for all to complete, aggregate results.

### Monitor

Polling loop with timer-based backoff — check a condition, sleep, repeat until done or timed out.

### Human Interaction

Wait for an external event (webhook, approval) with a timeout. If no response, escalate.

### Async HTTP

Start a workflow, get an instance ID, poll for status. The workflow runs in the background.

## SDK Examples

### TypeScript / Node.js

```typescript
import { AgentKernel } from "agentkernel";

const client = new AgentKernel();

// Define a workflow
const etl = client.durableFunction("etl-pipeline", async (ctx) => {
  // Step 1: Fetch data (runs in a sandbox)
  const raw = await ctx.callActivity("fetch", {
    image: "python:3.12-slim",
    cmd: ["python", "-c", "import json; print(json.dumps({'rows': 1000}))"],
  });

  // Step 2: Fan-out processing (parallel sandboxes)
  const chunks = [raw.rows.slice(0, 500), raw.rows.slice(500)];
  const processed = await ctx.all(
    chunks.map((chunk, i) =>
      ctx.callActivity(`process-${i}`, {
        image: "python:3.12-slim",
        cmd: ["python", "process.py"],
        input: chunk,
      })
    )
  );

  // Step 3: Aggregate (single sandbox)
  return await ctx.callActivity("aggregate", {
    cmd: ["python", "aggregate.py"],
    input: processed,
  });
});

// Start an instance
const instance = await etl.start({ input: { url: "https://data.example.com" } });
console.log(instance.id); // UUID of this run

// Check status
const status = await instance.getStatus();
// { status: "running", currentActivity: "process-1", completedActivities: 1 }

// Wait for completion
const result = await instance.waitForCompletion({ timeout: "10m" });
```

#### Fan-out / Fan-in

```typescript
const parallel = client.durableFunction("parallel-agents", async (ctx) => {
  const tasks = ["review", "test", "lint", "security-scan"];

  const results = await ctx.all(
    tasks.map((task) =>
      ctx.callActivity(task, {
        image: "node:22-slim",
        cmd: ["npx", task, "--json"],
        secrets: ["GITHUB_TOKEN:api.github.com"],
      })
    )
  );

  // All 4 sandboxes ran in parallel — aggregate results
  const allPassed = results.every((r) => r.passed);
  return { allPassed, results };
});
```

#### Timer + External Event

```typescript
const approval = client.durableFunction("deploy-approval", async (ctx) => {
  await ctx.callActivity("request-review", {
    cmd: ["python", "send_slack.py", "--channel", "deploys"],
  });

  // Wait up to 1 hour for human approval
  const event = await ctx.waitForExternalEvent("approved", {
    timeout: "1h",
  });

  if (event) {
    return await ctx.callActivity("deploy", {
      cmd: ["bash", "deploy.sh", "--prod"],
    });
  } else {
    return { status: "timed-out", escalated: true };
  }
});

// Later, from a webhook handler:
await instance.raiseEvent("approved", { reviewer: "alice" });
```

### Python

```python
from agentkernel import AgentKernel

client = AgentKernel()

@client.durable_function("etl-pipeline")
async def etl(ctx):
    # Step 1: Fetch
    raw = await ctx.call_activity("fetch", image="python:3.12-slim",
        cmd=["python", "-c", "import json; print(json.dumps({'rows': 1000}))"])

    # Step 2: Fan-out
    chunks = [raw["rows"][:500], raw["rows"][500:]]
    processed = await ctx.all([
        ctx.call_activity(f"process-{i}", image="python:3.12-slim",
            cmd=["python", "process.py"], input=chunk)
        for i, chunk in enumerate(chunks)
    ])

    # Step 3: Aggregate
    return await ctx.call_activity("aggregate",
        cmd=["python", "aggregate.py"], input=processed)

# Start
instance = await etl.start(input={"url": "https://data.example.com"})
result = await instance.wait_for_completion(timeout="10m")
```

#### Monitor Pattern

```python
@client.durable_function("health-monitor")
async def monitor(ctx):
    while True:
        status = await ctx.call_activity("check-health",
            cmd=["curl", "-sf", "https://api.example.com/health"])

        if status.get("degraded"):
            await ctx.call_activity("alert",
                cmd=["python", "send_alert.py", "--severity", "warn"])

        # Sleep 5 minutes without consuming resources
        await ctx.create_timer(minutes=5)
```

### Go

```go
client := agentkernel.New()

etl := client.DurableFunction("etl-pipeline", func(ctx *agentkernel.OrchestrationContext) (any, error) {
    // Step 1
    raw, err := ctx.CallActivity("fetch", agentkernel.ActivityOptions{
        Image: "python:3.12-slim",
        Cmd:   []string{"python", "fetch.py"},
    })
    if err != nil { return nil, err }

    // Step 2: Fan-out
    tasks := make([]agentkernel.ActivityCall, 2)
    tasks[0] = ctx.CallActivityAsync("process-0", agentkernel.ActivityOptions{
        Cmd: []string{"python", "process.py"}, Input: raw[:500],
    })
    tasks[1] = ctx.CallActivityAsync("process-1", agentkernel.ActivityOptions{
        Cmd: []string{"python", "process.py"}, Input: raw[500:],
    })
    results, err := ctx.All(tasks)
    if err != nil { return nil, err }

    // Step 3: Aggregate
    return ctx.CallActivity("aggregate", agentkernel.ActivityOptions{
        Cmd: []string{"python", "aggregate.py"}, Input: results,
    })
})

instance, _ := etl.Start(ctx, map[string]any{"url": "https://data.example.com"})
result, _ := instance.WaitForCompletion(ctx, 10*time.Minute)
```

### Rust

```rust
let client = AgentKernel::new(None)?;

let etl = client.durable_function("etl-pipeline", |ctx| async move {
    // Step 1
    let raw = ctx.call_activity("fetch", ActivityOptions {
        image: Some("python:3.12-slim"),
        cmd: vec!["python", "fetch.py"],
        ..Default::default()
    }).await?;

    // Step 2: Fan-out
    let results = ctx.all(vec![
        ctx.call_activity_async("process-0", ActivityOptions {
            cmd: vec!["python", "process.py"],
            input: Some(raw[..500].into()),
            ..Default::default()
        }),
        ctx.call_activity_async("process-1", ActivityOptions {
            cmd: vec!["python", "process.py"],
            input: Some(raw[500..].into()),
            ..Default::default()
        }),
    ]).await?;

    // Step 3: Aggregate
    ctx.call_activity("aggregate", ActivityOptions {
        cmd: vec!["python", "aggregate.py"],
        input: Some(results.into()),
        ..Default::default()
    }).await
}).await?;

let instance = etl.start(json!({"url": "https://data.example.com"})).await?;
let result = instance.wait_for_completion(Duration::from_secs(600)).await?;
```

### Swift

```swift
let client = AgentKernel()

let etl = try client.durableFunction("etl-pipeline") { ctx in
    // Step 1
    let raw = try await ctx.callActivity("fetch", options: .init(
        image: "python:3.12-slim",
        cmd: ["python", "fetch.py"]
    ))

    // Step 2: Fan-out
    let results = try await ctx.all([
        ctx.callActivityAsync("process-0", options: .init(
            cmd: ["python", "process.py"], input: raw[0..<500]
        )),
        ctx.callActivityAsync("process-1", options: .init(
            cmd: ["python", "process.py"], input: raw[500...]
        )),
    ])

    // Step 3
    return try await ctx.callActivity("aggregate", options: .init(
        cmd: ["python", "aggregate.py"], input: results
    ))
}

let instance = try await etl.start(input: ["url": "https://data.example.com"])
let result = try await instance.waitForCompletion(timeout: .minutes(10))
```

## HTTP API

```
POST   /orchestrations                    Start a new orchestration instance
GET    /orchestrations                    List all instances
GET    /orchestrations/{id}               Get instance status
POST   /orchestrations/{id}/events/{name} Raise an external event
POST   /orchestrations/{id}/retry/{activity-id} Retry a failed activity
DELETE /orchestrations/{id}               Terminate instance
GET    /orchestrations/{id}/history       Activity history
```

### Start an orchestration

```bash
curl -X POST http://localhost:18888/orchestrations \
  -H 'Content-Type: application/json' \
  -d '{"name": "etl-pipeline", "input": {"url": "https://data.example.com"}}'

# Response:
# {"id": "019abc12-...", "status": "running"}
```

### Check status

```bash
curl http://localhost:18888/orchestrations/019abc12-...

# {"id": "019abc12-...", "status": "running",
#  "current_activity": "process-1",
#  "completed_activities": 2,
#  "started_at": "2026-02-15T10:00:00Z"}
```

### Raise external event

```bash
curl -X POST http://localhost:18888/orchestrations/019abc12-.../events/approved \
  -H 'Content-Type: application/json' \
  -d '{"reviewer": "alice", "approved": true}'
```

## Cron Scheduling

Run orchestrations on a schedule via the daemon:

```toml
# agentkernel.toml
[[schedule]]
name = "nightly-etl"
cron = "0 3 * * *"
orchestration = "etl-pipeline"
input = '{"url": "https://data.example.com/daily"}'
```

## How Checkpointing Works

1. Orchestrator calls `ctx.callActivity("step-1", ...)`
2. Server runtime schedules the activity in a sandbox and captures a structured result envelope
3. Runtime appends an event to durable storage in `~/.local/share/agentkernel/state.db` (`history` table)
4. Orchestrator logic advances deterministically from ordered history rows
5. If SDK/server crashes after step 1:
   - Runtime reloads history on recovery
   - Step 1 is treated as completed from history
   - Pending work continues without closure/stack serialization

Checkpoint file format:

```json
[
  {
    "type": "activity_completed",
    "name": "step-1",
    "result": {"ok": true},
    "logs": [{"stream": "stdout", "chunk": "..."}],
    "metrics": {"duration_ms": 243, "exit_code": 0},
    "ts": "..."
  },
  {"type": "timer_fired", "fire_at": "...", "ts": "..."},
  {"type": "event_received", "name": "approved", "data": {...}, "ts": "..."}
]
```

## Execution Guarantees

- Activities are delivered **at least once**; handlers should be idempotent.
- Runtime provides an idempotency key per activity attempt.
- Fan-out is subject to server-side quota and backpressure limits.
- Large logs/results are size-limited and may be truncated with artifact references.

## See Also

- [Durable Actors](durable-objects.md) — Stateful sandbox actors with hibernation
- [Browser Automation](browser-automation.md) — Similar SDK wrapper pattern
- [Secrets](secrets.md) — Activities can use secret bindings
