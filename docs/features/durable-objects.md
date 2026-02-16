
# Durable Actors (Durable Objects-style)

Stateful sandboxes that persist state across calls, auto-hibernate when idle, and wake on demand. Each object is a sandbox with an in-sandbox HTTP server that handles method dispatch — the same pattern as [browser automation](browser-automation.md).

Inspired by [Cloudflare Durable Objects](https://developers.cloudflare.com/durable-objects/), with VM-level isolation instead of V8 isolates.

## Why Durable Objects?

Some workloads need persistent, addressable state: counters, session stores, coordination locks, caches. Durable Objects give each stateful entity its own sandbox with:

- **Serialized execution** — one in-flight method call per object (queued FIFO)
- **Persistent state** — filesystem survives stop/start cycles
- **Auto-hibernation** — stops the VM when idle, restarts on next call
- **Alarms** — scheduled method calls that wake the object
- **VM isolation** — each object runs in its own Linux kernel

## Core Concepts

| Concept | Description |
|---------|-------------|
| **Object** | A sandbox running an HTTP server with named methods |
| **Method** | A named handler that reads/writes state and returns a result |
| **State** | JSON stored on the sandbox filesystem at `/data/state.json` |
| **Hibernation** | Sandbox stopped after idle timeout, restarted on next call |
| **Alarm** | Scheduled future method call that wakes the object |
| **UUID** | Globally unique identifier (UUIDv7) for addressing at scale |

## Durable Protocol (Cross-SDK)

Durable Objects share the same protocol contract as Durable Functions:

- Canonical IDs (`object_id`, `event_id`, `alarm_id`)
- Canonical call/result envelopes
- Versioned schema with backward-compatible evolution rules

See [Durable Protocol](durable-protocol.md) for common envelope details.

## SDK Examples

### TypeScript / Node.js

```typescript
import { AgentKernel } from "agentkernel";

const client = new AgentKernel();

// Create a durable object
const counter = await client.durableObject("visit-counter", {
  image: "node:22-slim",
  memory_mb: 256,
  idle_timeout: "5m",
  state: { count: 0, last_visitor: null },
  handlers: {
    increment: `(input, state) => {
      state.count += input.by || 1;
      state.last_visitor = input.visitor;
      return { count: state.count };
    }`,
    get: `(input, state) => state`,
    reset: `(input, state) => {
      state.count = 0;
      state.last_visitor = null;
      return state;
    }`,
  },
});

// Call methods — auto-wakes if hibernated
const result = await counter.call("increment", { by: 1, visitor: "alice" });
console.log(result); // { count: 1 }

const state = await counter.call("get");
console.log(state); // { count: 1, last_visitor: "alice" }

// Set an alarm
await counter.setAlarm({
  at: new Date("2026-03-01T00:00:00Z"),
  method: "reset",
});

// Check object info
const info = await counter.info();
console.log(info.uuid);    // "019abc12-..."
console.log(info.status);  // "active" | "hibernated"

// Hibernate explicitly
await counter.hibernate();

// Next call() auto-wakes it
const fresh = await counter.call("get"); // sandbox restarts transparently

// Delete permanently
await counter.delete();
```

#### Coordination Lock

```typescript
const lock = await client.durableObject("deploy-lock", {
  image: "node:22-slim",
  idle_timeout: "10m",
  state: { locked: false, owner: null, acquired_at: null },
  handlers: {
    acquire: `(input, state) => {
      if (state.locked) return { acquired: false, owner: state.owner };
      state.locked = true;
      state.owner = input.owner;
      state.acquired_at = new Date().toISOString();
      return { acquired: true };
    }`,
    release: `(input, state) => {
      if (state.owner !== input.owner) return { released: false };
      state.locked = false;
      state.owner = null;
      state.acquired_at = null;
      return { released: true };
    }`,
    status: `(input, state) => state`,
  },
});

const result = await lock.call("acquire", { owner: "agent-1" });
if (result.acquired) {
  // Do work...
  await lock.call("release", { owner: "agent-1" });
}
```

#### Session Store

```typescript
const session = await client.durableObject("session-user-123", {
  image: "node:22-slim",
  idle_timeout: "30m",
  state: { cart: [], preferences: {} },
  handlers: {
    addToCart: `(input, state) => {
      state.cart.push(input.item);
      return { cart_size: state.cart.length };
    }`,
    getCart: `(input, state) => ({ cart: state.cart })`,
    setPreference: `(input, state) => {
      state.preferences[input.key] = input.value;
      return state.preferences;
    }`,
    checkout: `(input, state) => {
      const order = { items: [...state.cart], total: state.cart.length };
      state.cart = [];
      return order;
    }`,
  },
});
```

### Python

```python
from agentkernel import AgentKernel

client = AgentKernel()

# Create a durable object
counter = await client.durable_object("visit-counter",
    image="node:22-slim",
    memory_mb=256,
    idle_timeout="5m",
    state={"count": 0, "last_visitor": None},
    handlers={
        "increment": """(input, state) => {
            state.count += input.by || 1;
            state.last_visitor = input.visitor;
            return { count: state.count };
        }""",
        "get": "(input, state) => state",
        "reset": """(input, state) => {
            state.count = 0;
            state.last_visitor = null;
            return state;
        }""",
    },
)

# Call methods
result = await counter.call("increment", {"by": 1, "visitor": "alice"})
state = await counter.call("get")

# Alarm
await counter.set_alarm(
    at="2026-03-01T00:00:00Z",
    method="reset",
)

# Lifecycle
await counter.hibernate()
await counter.call("get")  # auto-wakes
await counter.delete()
```

#### Rate Limiter

```python
limiter = await client.durable_object("rate-limit-api-key-xyz",
    image="node:22-slim",
    idle_timeout="1m",
    state={"requests": [], "limit": 100, "window_seconds": 60},
    handlers={
        "check": """(input, state) => {
            const now = Date.now();
            const window = state.window_seconds * 1000;
            state.requests = state.requests.filter(t => now - t < window);
            if (state.requests.length >= state.limit) {
                return { allowed: false, retry_after: Math.ceil((state.requests[0] + window - now) / 1000) };
            }
            state.requests.push(now);
            return { allowed: true, remaining: state.limit - state.requests.length };
        }""",
        "reset": "(input, state) => { state.requests = []; return { reset: true }; }",
    },
)

result = await limiter.call("check")
if result["allowed"]:
    # proceed
    pass
else:
    print(f"Rate limited, retry in {result['retry_after']}s")
```

### Go

```go
client := agentkernel.New()

counter, err := client.DurableObject(ctx, "visit-counter", agentkernel.ObjectOptions{
    Image:       "node:22-slim",
    MemoryMB:    256,
    IdleTimeout: 5 * time.Minute,
    State:       map[string]any{"count": 0, "last_visitor": nil},
    Handlers: map[string]string{
        "increment": `(input, state) => {
            state.count += input.by || 1;
            state.last_visitor = input.visitor;
            return { count: state.count };
        }`,
        "get":   "(input, state) => state",
        "reset": "(input, state) => { state.count = 0; return state; }",
    },
})
if err != nil { log.Fatal(err) }
defer counter.Close()

result, _ := counter.Call(ctx, "increment", map[string]any{"by": 1, "visitor": "alice"})
fmt.Println(result) // map[count:1]

state, _ := counter.Call(ctx, "get", nil)

counter.SetAlarm(ctx, time.Date(2026, 3, 1, 0, 0, 0, 0, time.UTC), "reset", nil)

counter.Hibernate(ctx)
counter.Call(ctx, "get", nil) // auto-wakes
counter.Delete(ctx)
```

### Rust

```rust
let client = AgentKernel::new(None)?;

let counter = client.durable_object("visit-counter", ObjectOptions {
    image: Some("node:22-slim".into()),
    memory_mb: Some(256),
    idle_timeout: Some(Duration::from_secs(300)),
    state: Some(json!({"count": 0, "last_visitor": null})),
    handlers: HashMap::from([
        ("increment".into(), r#"(input, state) => {
            state.count += input.by || 1;
            state.last_visitor = input.visitor;
            return { count: state.count };
        }"#.into()),
        ("get".into(), "(input, state) => state".into()),
        ("reset".into(), "(input, state) => { state.count = 0; return state; }".into()),
    ]),
}).await?;

let result = counter.call("increment", json!({"by": 1, "visitor": "alice"})).await?;
let state = counter.call("get", json!({})).await?;

counter.set_alarm(
    Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap(),
    "reset",
    None,
).await?;

counter.hibernate().await?;
counter.call("get", json!({})).await?; // auto-wakes
counter.delete().await?;
```

### Swift

```swift
let client = AgentKernel()

let counter = try await client.durableObject("visit-counter", options: .init(
    image: "node:22-slim",
    memoryMB: 256,
    idleTimeout: .minutes(5),
    state: ["count": 0, "last_visitor": nil],
    handlers: [
        "increment": """
            (input, state) => {
                state.count += input.by || 1;
                state.last_visitor = input.visitor;
                return { count: state.count };
            }
            """,
        "get": "(input, state) => state",
        "reset": "(input, state) => { state.count = 0; return state; }",
    ]
))

let result = try await counter.call("increment", input: ["by": 1, "visitor": "alice"])
let state = try await counter.call("get")

try await counter.setAlarm(
    at: DateComponents(year: 2026, month: 3, day: 1),
    method: "reset"
)

try await counter.hibernate()
try await counter.call("get") // auto-wakes
try await counter.delete()
```

## HTTP API

```
POST   /objects                          Create a durable object
GET    /objects                          List objects
GET    /objects/{id}                     Object info + status
POST   /objects/{id}/call/{method}       Call a method
GET    /objects/{id}/state               Read current state
POST   /objects/{id}/hibernate           Force hibernate
POST   /objects/{id}/wake                Force wake
POST   /objects/{id}/alarms              Set an alarm
GET    /objects/{id}/alarms              List alarms
DELETE /objects/{id}/alarms/{alarm-id}   Cancel an alarm
DELETE /objects/{id}                     Delete object + state
```

## Execution Guarantees

- Method calls are processed one-at-a-time per object.
- Calls are queued FIFO by default.
- Alarm delivery is at least once.
- Each call includes an idempotency key for safe retries.

## Durability Modes

Durability mode should be explicit in object configuration:

- `ephemeral`: state removed with sandbox lifecycle
- `restart_persistent`: state survives stop/start in sandbox filesystem
- `durable_snapshot`: state persisted via snapshots on hibernate/checkpoint

### Create an object

```bash
curl -X POST http://localhost:18888/objects \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "visit-counter",
    "image": "node:22-slim",
    "idle_timeout": "5m",
    "state": {"count": 0},
    "handlers": {
      "increment": "(input, state) => { state.count++; return state; }",
      "get": "(input, state) => state"
    }
  }'

# {"uuid": "019abc12-...", "name": "visit-counter", "status": "active"}
```

### Call a method

```bash
curl -X POST http://localhost:18888/objects/visit-counter/call/increment \
  -H 'Content-Type: application/json' \
  -d '{"by": 5}'

# {
#   "result": {"count": 5},
#   "metrics": {"duration_ms": 12},
#   "idempotency_key": "..."
# }
```

### Set an alarm

```bash
curl -X POST http://localhost:18888/objects/visit-counter/alarms \
  -H 'Content-Type: application/json' \
  -d '{"at": "2026-03-01T00:00:00Z", "method": "reset"}'

# {"alarm_id": "019def34-...", "fires_at": "2026-03-01T00:00:00Z"}
```

## How It Works

### In-Sandbox Server

When you create a durable object, the SDK:

1. Creates a sandbox with the specified image
2. Writes the handler script + state to the sandbox filesystem
3. Starts an HTTP server inside the sandbox on port 9333
4. The server loads state from `/data/state.json`, dispatches method calls, saves state back

This is the same pattern as [browser automation](browser-automation.md), which runs a Playwright server on port 9222.

### Hibernation

When `idle_timeout` expires:

1. The in-sandbox server flushes state to `/data/state.json`
2. The sandbox is stopped (but not removed)
3. Container filesystem is preserved

On next `call()`:

1. The sandbox is started
2. The server boots and loads state from `/data/state.json`
3. The method call is dispatched
4. Idle timeout resets

### Alarms

Alarms are persisted in `~/.local/share/agentkernel/alarms/`:

```json
{
  "alarm_id": "019def34-...",
  "object_name": "visit-counter",
  "fires_at": "2026-03-01T00:00:00Z",
  "method": "reset",
  "input": null
}
```

The daemon checks alarms every minute. When one fires:
1. Wake the object (start sandbox if hibernated)
2. Call the method
3. Remove the alarm (or reschedule if recurring)

## Cron Scheduling

Run periodic method calls via the daemon:

```toml
# agentkernel.toml
[[schedule]]
name = "reset-counters"
cron = "0 0 * * *"        # midnight daily
object = "visit-counter"
method = "reset"
```

## Comparison with Browser Sessions

| Aspect | Browser Session | Durable Object |
|--------|----------------|----------------|
| In-sandbox server | Playwright on port 9222 | Custom handlers on port 9333 |
| State | DOM / pages | JSON on filesystem |
| Methods | open, click, fill, snapshot | User-defined |
| Hibernation | No (always running) | Yes (auto stop/start) |
| Alarms | No | Yes |
| Use case | Web automation | Stateful services |

## See Also

- [Durable Functions](durable-functions.md) — Orchestrated workflows with checkpointing
- [Browser Automation](browser-automation.md) — Same in-sandbox server pattern
- [Secrets](secrets.md) — Objects can use secret bindings for API access
