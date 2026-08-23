
# HTTP API

agentkernel includes a REST API for programmatic sandbox management.

## Starting the Server

```bash
# As a background service (recommended — survives reboots)
brew services start thrashr888/agentkernel/agentkernel

# Or start manually on default port (18888)
agentkernel serve

# Custom port
agentkernel serve --port 3000

# With API key authentication
agentkernel serve --api-key your-secret

# Multiple API keys
agentkernel serve --api-key key1 --api-key key2

# Load keys from a file (one per line, # comments supported)
agentkernel serve --api-key-file /path/to/keys.txt

# Or via environment variable
AGENTKERNEL_API_KEY=your-secret agentkernel serve

# With OpenTelemetry trace export
agentkernel serve --otel-endpoint http://localhost:4318

# With webhook notifications
agentkernel serve --webhook-url http://localhost:9999/hooks

# Multiple webhooks + OTel
agentkernel serve --otel-endpoint http://localhost:4318 \
  --webhook-url http://hook1.example.com \
  --webhook-url http://hook2.example.com
```

## Authentication

When API key authentication is enabled (via `--api-key`, `--api-key-file`, `AGENTKERNEL_API_KEY`, or `[api].api_key` in config), all requests require an `Authorization` header, except for `/health` and `/status`:

```text
Authorization: Bearer your-secret
```

```bash
curl -H "Authorization: Bearer your-secret" http://localhost:18888/sandboxes
```

Multiple keys can be configured for key rotation or multi-tenant setups. Any valid key will authenticate the request.

## Endpoints

### Health Check

```
GET /health
```

```bash
curl http://localhost:18888/health
```

```json
{"status": "ok"}
```

### Server Status

```
GET /status
```

```bash
curl http://localhost:18888/status
```

```json
{
  "success": true,
  "data": {"version": "0.15.0", "backend": "docker", "api_key_configured": false}
}
```

### Server Statistics

```
GET /stats
```

```bash
curl http://localhost:18888/stats
```

```json
{
  "success": true,
  "data": {
    "sandbox_count": 12,
    "sandbox_limit": 0,
    "backend": "docker",
    "uptime_seconds": 3600,
    "version": "0.15.0",
    "resource_usage": {
      "cpu_percent": 65.2,
      "memory_used_mb": 8192,
      "memory_total_mb": 16384,
      "disk_used_mb": 4096
    }
  }
}
```

The `resource_usage` field provides host-level CPU, memory, and disk metrics for fleet load-balancing.

### Event Stream (SSE)

```
GET /events
```

Streams sandbox lifecycle events via Server-Sent Events. Requires authentication when API keys are configured. Requires `--webhook-url` or `--otel-endpoint` to enable the event bus.

```bash
# Stream all events
curl -N http://localhost:18888/events

# Filter to a specific sandbox
curl -N http://localhost:18888/events?sandbox=my-sandbox
```

Events:
- `sandbox.created` — sandbox was created and started
- `sandbox.exec.completed` — command execution finished (includes `exit_code`, `duration_ms`)
- `sandbox.deleted` — sandbox was removed

Event payload:
```json
{
  "event": "sandbox.exec.completed",
  "timestamp": "2026-02-23T12:00:00Z",
  "sandbox": "my-sandbox",
  "labels": {},
  "metadata": {
    "command": "echo hello",
    "duration_ms": 42,
    "success": true,
    "exit_code": 0
  }
}
```

### Observability Flags

| Flag | Description |
|------|-------------|
| `--otel-endpoint URL` | OTLP/HTTP endpoint for trace export (e.g. `http://localhost:4318`) |
| `--webhook-url URL` | POST events to this URL (can be repeated) |

When `--otel-endpoint` is set, every HTTP request creates an OTel span with W3C `traceparent` propagation. Pass a `traceparent` header on incoming requests to link sandbox operations to your existing traces.

When executing commands (`POST /sandboxes/{name}/exec`), the `TRACEPARENT` and `TRACESTATE` environment variables are automatically injected into the sandbox, enabling code inside the sandbox to continue the distributed trace.

### Garbage Collection

```
POST /gc
```

```bash
curl -X POST http://localhost:18888/gc
```

```json
{
  "success": true,
  "data": {"removed": ["expired-sandbox-1", "old-test"]}
}
```

Removes sandboxes that have exceeded their time-to-live.

### Run Command

Execute a command in a temporary sandbox.

```
POST /run
```

```bash
curl -X POST http://localhost:18888/run \
  -H "Content-Type: application/json" \
  -d '{"command": ["python3", "-c", "print(1+1)"]}'
```

```json
{
  "success": true,
  "data": {"output": "2\n"}
}
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `command` | array | Yes | Command and arguments |
| `image` | string | No | Docker image (auto-detected if omitted) |
| `profile` | string | No | Security profile |
| `fast` | bool | No | Use container pool (default: true) |

### Run Command (Streaming)

Execute a command with Server-Sent Events (SSE) streaming.

```
POST /run/stream
```

```bash
curl -X POST http://localhost:18888/run/stream \
  -H "Content-Type: application/json" \
  -d '{"command": ["python3", "long_script.py"]}'
```

**Response (SSE stream):**

```
event: started
data: {"sandbox":"sandbox-abc123"}

event: progress
data: {"stage":"creating"}

event: progress
data: {"stage":"starting"}

event: progress
data: {"stage":"executing"}

event: output
data: {"content":"Processing step 1...\n"}

event: output
data: {"content":"Processing step 2...\n"}

event: done
data: {"exit_code":0}
```

**Event types:**

| Event | Data | Description |
|-------|------|-------------|
| `started` | `{"sandbox": "name"}` | Command execution started |
| `progress` | `{"stage": "..."}` | Execution stage (creating, starting, executing) |
| `output` | `{"content": "..."}` | Command output (stdout/stderr) |
| `done` | `{"exit_code": 0}` | Command completed successfully |
| `error` | `{"message": "..."}` | Error occurred |

**Request body:** Same as `/run`

**Use cases:**
- Long-running commands
- Real-time output display
- Progress tracking

### List Sandboxes

```
GET /sandboxes
GET /sandboxes?label=key:value
```

```bash
# List all sandboxes
curl http://localhost:18888/sandboxes

# Filter by labels (multiple labels are ANDed)
curl "http://localhost:18888/sandboxes?label=env:prod&label=team:ml"
```

```json
{
  "success": true,
  "data": [
    {"name": "my-sandbox", "uuid": "019abc12-1234-7def-89ab-0123456789ab", "status": "running", "backend": "docker", "ip": "172.17.0.3"},
    {"name": "test", "uuid": "019abc12-2345-7def-89ab-0123456789ab", "status": "stopped", "backend": "docker"}
  ]
}
```

The `ip` field contains the container's Docker bridge network IP address. It is only present for running sandboxes.

### Create Sandbox

```
POST /sandboxes
```

```bash
curl -X POST http://localhost:18888/sandboxes \
  -H "Content-Type: application/json" \
  -d '{"name": "my-sandbox", "image": "python:3.12-alpine"}'
```

```json
{
  "success": true,
  "data": {"name": "my-sandbox", "uuid": "019abc12-1234-7def-89ab-0123456789ab", "status": "running", "backend": "docker"}
}
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Sandbox name |
| `image` | string | No | Docker image (auto-detected if omitted) |
| `vcpus` | integer | No | Number of vCPUs (default: 1) |
| `memory_mb` | integer | No | Memory in MB (default: 512) |
| `profile` | string | No | Security profile: `permissive`, `moderate`, `restrictive` |
| `labels` | object | No | Key-value labels for fleet management and filtering |
| `description` | string | No | Human-readable description |
| `lifecycle` | object | No | Lifecycle policy (`auto_stop_after_seconds`, `auto_archive_after_seconds`, `auto_delete_after_seconds`) |

**With labels and description:**

```bash
curl -X POST http://localhost:18888/sandboxes \
  -H "Content-Type: application/json" \
  -d '{
    "name": "eval-sandbox",
    "image": "python:3.12-alpine",
    "labels": {"scenario": "drift_s3", "model": "sonnet", "eval_run": "pr-123"},
    "description": "Drift scenario evaluation"
  }'
```

### Update Sandbox

```
PATCH /sandboxes/{name}
```

```bash
curl -X PATCH http://localhost:18888/sandboxes/my-sandbox \
  -H "Content-Type: application/json" \
  -d '{"labels": {"env": "staging"}, "description": "Updated description"}'
```

```json
{
  "success": true,
  "data": {"name": "my-sandbox", "uuid": "019abc12-...", "status": "running", "backend": "docker"}
}
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `labels` | object | No | Replace all labels |
| `description` | string | No | Update description |
| `lifecycle` | object or null | No | Set lifecycle policy or clear it with `null` |

### Get Sandbox

```
GET /sandboxes/{name}
```

```bash
curl http://localhost:18888/sandboxes/my-sandbox
```

```json
{
  "success": true,
  "data": {
    "name": "my-sandbox",
    "uuid": "019abc12-1234-7def-89ab-0123456789ab",
    "status": "running",
    "backend": "docker",
    "ip": "172.17.0.3",
    "image": "python:3.12-alpine",
    "vcpus": 1,
    "memory_mb": 512,
    "created_at": "2026-01-30T12:00:00Z"
  }
}
```

The response includes resource limits and metadata when available. The `ip` field is only present for running sandboxes. Fields that are unknown are omitted.

### Get Sandbox by UUID

```
GET /sandboxes/by-uuid/{uuid}
```

```bash
curl http://localhost:18888/sandboxes/by-uuid/019abc12-1234-7def-89ab-0123456789ab
```

### Execute in Sandbox

```
POST /sandboxes/{name}/exec
```

```bash
curl -X POST http://localhost:18888/sandboxes/my-sandbox/exec \
  -H "Content-Type: application/json" \
  -d '{"command": ["ls", "-la"]}'
```

```json
{
  "success": true,
  "data": {"output": "total 0\ndrwxr-xr-x..."}
}
```

### Stop Sandbox

```
POST /sandboxes/{name}/stop
```

```bash
curl -X POST http://localhost:18888/sandboxes/my-sandbox/stop
```

### Delete Sandbox

```
DELETE /sandboxes/{name}
```

```bash
curl -X DELETE http://localhost:18888/sandboxes/my-sandbox
```

### Extend TTL

Extend a sandbox's time-to-live.

```
POST /sandboxes/{name}/extend
```

```bash
curl -X POST http://localhost:18888/sandboxes/my-sandbox/extend \
  -H "Content-Type: application/json" \
  -d '{"by": "1h"}'
```

```json
{
  "success": true,
  "data": {"expires_at": "2026-02-05T15:00:00Z"}
}
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `by` | string | No | Duration to extend (default: "1h"). Examples: "30m", "2h", "1d" |

### Recover Archived Sandbox

Clears archive metadata so an archived sandbox can be started again.

```
POST /sandboxes/{name}/recover
```

```bash
curl -X POST http://localhost:18888/sandboxes/my-sandbox/recover
```

### Reconcile Lifecycle Policies

Applies lifecycle policies across all sandboxes (or previews actions).

```
POST /lifecycle/reconcile
```

```bash
# Apply lifecycle actions
curl -X POST http://localhost:18888/lifecycle/reconcile

# Dry run (preview only)
curl -X POST http://localhost:18888/lifecycle/reconcile \
  -H "Content-Type: application/json" \
  -d '{"dry_run": true}'
```

### File Operations

Read, write, and delete files inside a running sandbox.

#### Write File

```
PUT /sandboxes/{name}/files/{path...}
```

```bash
curl -X PUT http://localhost:18888/sandboxes/my-sandbox/files/tmp/hello.txt \
  -H "Content-Type: application/json" \
  -d '{"content": "hello world"}'
```

```json
{
  "success": true,
  "data": "Wrote 11 bytes to /tmp/hello.txt"
}
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | string | Yes | File content (text or base64-encoded) |
| `encoding` | string | No | `utf8` (default) or `base64` |

**Binary file (base64):**

```bash
curl -X PUT http://localhost:18888/sandboxes/my-sandbox/files/tmp/data.bin \
  -H "Content-Type: application/json" \
  -d '{"content": "aGVsbG8=", "encoding": "base64"}'
```

#### Read File

```
GET /sandboxes/{name}/files/{path...}
```

```bash
curl http://localhost:18888/sandboxes/my-sandbox/files/tmp/hello.txt
```

```json
{
  "success": true,
  "data": {
    "content": "hello world",
    "encoding": "utf8",
    "size": 11
  }
}
```

Binary files are returned as base64 with `"encoding": "base64"`.

#### Delete File

```
DELETE /sandboxes/{name}/files/{path...}
```

```bash
curl -X DELETE http://localhost:18888/sandboxes/my-sandbox/files/tmp/hello.txt
```

```json
{
  "success": true,
  "data": "Deleted /tmp/hello.txt"
}
```

### Sandbox Logs

Retrieve audit log entries for a specific sandbox.

```
GET /sandboxes/{name}/logs
```

```bash
curl http://localhost:18888/sandboxes/my-sandbox/logs
```

```json
{
  "success": true,
  "data": [
    {
      "timestamp": "2026-01-30T12:00:00Z",
      "event": "sandbox_created",
      "sandbox": "my-sandbox"
    }
  ]
}
```

Returns all audit events associated with the sandbox, sorted by timestamp. See [audit logging](../commands/index.md#audit-logging) for event types.

### Batch Execution

Run multiple commands in parallel, each in its own temporary sandbox.

```
POST /batch/run
```

```bash
curl -X POST http://localhost:18888/batch/run \
  -H "Content-Type: application/json" \
  -d '{
    "commands": [
      {"command": ["echo", "hello"]},
      {"command": ["python3", "-c", "print(2+2)"]}
    ]
  }'
```

```json
{
  "success": true,
  "data": {
    "results": [
      {"output": "hello\n", "error": null},
      {"output": "4\n", "error": null}
    ]
  }
}
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `commands` | array | Yes | List of commands to run |
| `commands[].command` | array | Yes | Command and arguments |

Each command runs in an isolated container from the pool. Results are returned in the same order as the input commands.

### Snapshots

#### List Snapshots

```
GET /snapshots
```

```bash
curl http://localhost:18888/snapshots
```

```json
{
  "success": true,
  "data": [
    {
      "name": "checkpoint-1",
      "sandbox": "my-sandbox",
      "image_tag": "agentkernel-snap:checkpoint-1",
      "backend": "docker",
      "base_image": "python:3.12-alpine",
      "vcpus": 2,
      "memory_mb": 512,
      "created_at": "2026-02-05T12:00:00Z"
    }
  ]
}
```

#### Take Snapshot

```
POST /snapshots
```

```bash
curl -X POST http://localhost:18888/snapshots \
  -H "Content-Type: application/json" \
  -d '{"sandbox": "my-sandbox", "name": "checkpoint-1"}'
```

```json
{
  "success": true,
  "data": {
    "name": "checkpoint-1",
    "sandbox": "my-sandbox",
    "image_tag": "agentkernel-snap:checkpoint-1",
    "backend": "docker",
    "base_image": "python:3.12-alpine",
    "vcpus": 2,
    "memory_mb": 512,
    "created_at": "2026-02-05T12:00:00Z"
  }
}
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `sandbox` | string | Yes | Name of the sandbox to snapshot |
| `name` | string | Yes | Name for the snapshot |

#### Get Snapshot

```
GET /snapshots/{name}
```

```bash
curl http://localhost:18888/snapshots/checkpoint-1
```

Returns snapshot details (same format as list).

#### Delete Snapshot

```
DELETE /snapshots/{name}
```

```bash
curl -X DELETE http://localhost:18888/snapshots/checkpoint-1
```

```json
{
  "success": true,
  "data": "Snapshot deleted"
}
```

#### Restore Snapshot

```
POST /snapshots/{name}/restore
```

```bash
curl -X POST http://localhost:18888/snapshots/checkpoint-1/restore \
  -H "Content-Type: application/json" \
  -d '{"as_name": "restored-sandbox"}'
```

```json
{
  "success": true,
  "data": {
    "sandbox": "restored-sandbox",
    "from_snapshot": "checkpoint-1"
  }
}
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `as_name` | string | No | Name for the restored sandbox (defaults to `{original}-restored`) |

### Browser Automation

Control a persistent headless browser inside a sandbox via ARIA snapshots.

The browser server starts automatically on first use. It runs Chromium via Playwright inside the sandbox and exposes an internal HTTP API on port 9222.

#### Start Browser Server

```
POST /sandboxes/{name}/browser/start
```

```bash
curl -X POST http://localhost:18888/sandboxes/my-browser/browser/start
```

Starts the in-sandbox browser server. Called automatically by other browser endpoints if needed.

#### List Pages

```
GET /sandboxes/{name}/browser/pages
```

```bash
curl http://localhost:18888/sandboxes/my-browser/browser/pages
```

```json
{"pages": ["default", "docs"]}
```

#### Navigate (Goto)

```
POST /sandboxes/{name}/browser/pages/{page}/goto
```

```bash
curl -X POST http://localhost:18888/sandboxes/my-browser/browser/pages/default/goto \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

```json
{
  "snapshot": "- document \"Example Domain\":\n  - heading \"Example Domain\" [level=1] [ref=e1]\n  ...",
  "url": "https://example.com/",
  "title": "Example Domain",
  "refs": ["e1", "e2"]
}
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `url` | string | Yes | URL to navigate to |

#### Get ARIA Snapshot

```
GET /sandboxes/{name}/browser/pages/{page}/snapshot
```

```bash
curl http://localhost:18888/sandboxes/my-browser/browser/pages/default/snapshot
```

Returns the current ARIA snapshot without navigating. Same response format as goto.

#### Click Element

```
POST /sandboxes/{name}/browser/pages/{page}/click
```

```bash
curl -X POST http://localhost:18888/sandboxes/my-browser/browser/pages/default/click \
  -H "Content-Type: application/json" \
  -d '{"ref": "e2"}'
```

Returns a new ARIA snapshot after clicking.

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `ref` | string | No | Ref ID from ARIA snapshot |
| `selector` | string | No | CSS selector (fallback) |

#### Fill Input

```
POST /sandboxes/{name}/browser/pages/{page}/fill
```

```bash
curl -X POST http://localhost:18888/sandboxes/my-browser/browser/pages/default/fill \
  -H "Content-Type: application/json" \
  -d '{"ref": "e3", "value": "search query"}'
```

Returns a new ARIA snapshot after filling.

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `value` | string | Yes | Text to type |
| `ref` | string | No | Ref ID from ARIA snapshot |
| `selector` | string | No | CSS selector (fallback) |

#### Screenshot

```
POST /sandboxes/{name}/browser/pages/{page}/screenshot
```

Returns a PNG screenshot as base64.

#### Evaluate JavaScript

```
POST /sandboxes/{name}/browser/pages/{page}/evaluate
```

```bash
curl -X POST http://localhost:18888/sandboxes/my-browser/browser/pages/default/evaluate \
  -H "Content-Type: application/json" \
  -d '{"expression": "document.title"}'
```

#### Get Page Content

```
GET /sandboxes/{name}/browser/pages/{page}/content
```

Returns raw page content (title, text, links) — similar to v1 goto format.

#### Close Page

```
DELETE /sandboxes/{name}/browser/pages/{page}
```

```bash
curl -X DELETE http://localhost:18888/sandboxes/my-browser/browser/pages/default
```

#### Browser Events

```
GET /sandboxes/{name}/browser/events
```

```bash
curl "http://localhost:18888/sandboxes/my-browser/browser/events?offset=0&limit=50"
```

```json
[
  {"seq": 1, "type": "page.navigated", "page": "default", "ts": "2026-02-10T12:00:00Z"},
  {"seq": 2, "type": "page.clicked", "page": "default", "ts": "2026-02-10T12:00:01Z"}
]
```

**Query parameters:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `offset` | integer | No | Start from this sequence number (default: 0) |
| `limit` | integer | No | Max events to return (default: 100) |

### Durable Objects

#### List Objects

```
GET /objects
```

```bash
curl http://localhost:18888/objects
```

```json
{
  "success": true,
  "data": [
    {
      "id": "019abc12-...",
      "class": "Counter",
      "object_id": "counter-1",
      "status": "active",
      "sandbox": "my-sandbox",
      "storage": {"count": 42},
      "idle_timeout_seconds": 300,
      "created_at": "2026-02-18T12:00:00Z",
      "updated_at": "2026-02-18T12:00:00Z"
    }
  ]
}
```

#### Create Object

```
POST /objects
```

```bash
curl -X POST http://localhost:18888/objects \
  -H "Content-Type: application/json" \
  -d '{"class": "Counter", "object_id": "counter-1", "sandbox": "my-sandbox"}'
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `class` | string | Yes | Object class name |
| `object_id` | string | Yes | Unique object identifier within the class |
| `sandbox` | string | No | Sandbox to bind to (validated if provided) |
| `storage` | object | No | Initial storage state |
| `idle_timeout_seconds` | integer | No | Seconds before hibernation (default: 300) |

#### Get Object

```
GET /objects/{id}
```

```bash
curl http://localhost:18888/objects/019abc12-...
```

#### Update Object

```
PATCH /objects/{id}
```

```bash
curl -X PATCH http://localhost:18888/objects/019abc12-... \
  -H "Content-Type: application/json" \
  -d '{"storage": {"count": 99}}'
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `storage` | object | No | Replace storage state |
| `status` | string | No | Set status (`active`, `hibernating`) |

#### Delete Object

```
DELETE /objects/{id}
```

```bash
curl -X DELETE http://localhost:18888/objects/019abc12-...
```

#### Call Object Method

```
POST /objects/{class}/{object_id}/call/{method}
```

```bash
curl -X POST http://localhost:18888/objects/Counter/counter-1/call/increment \
  -H "Content-Type: application/json" \
  -d '{"amount": 1}'
```

Auto-creates the object if it does not exist. Wakes from hibernation if needed. The request body is passed as method arguments.

### Schedules

When `[[schedule]]` entries are present in `agentkernel.toml`, the
`/schedules/configured` endpoints list the daemon-integrated user jobs and
expose their truthful execution state. The config jobs use stable IDs and run
in UTC. `GET /schedules/configured/{id}` and `GET
/schedules/configured/{id}/status` return `enabled`, `status`, `last_run_at`,
`last_error`, and the derived `next_run_at`. `POST
/schedules/configured/{id}/trigger` executes one configured job immediately;
it does not require the current minute to match cron.

```bash
curl http://localhost:18888/schedules/configured
curl http://localhost:18888/schedules/configured/refresh-index/status
curl -X POST http://localhost:18888/schedules/configured/refresh-index/trigger
```

The existing `POST /schedules` CRUD form remains available for durable-object
schedule records. Config-defined jobs are validated at daemon startup and are
not mutated by the HTTP API.

#### List Schedules

```
GET /schedules
```

```bash
curl http://localhost:18888/schedules
```

```json
{
  "success": true,
  "data": [
    {
      "id": "019abc12-...",
      "name": "daily-cleanup",
      "cron": "0 0 * * *",
      "method": "cleanup",
      "args": {},
      "status": "active",
      "created_at": "2026-02-18T12:00:00Z",
      "updated_at": "2026-02-18T12:00:00Z"
    }
  ]
}
```

#### Create Schedule

```
POST /schedules
```

```bash
curl -X POST http://localhost:18888/schedules \
  -H "Content-Type: application/json" \
  -d '{"name": "daily-cleanup", "cron": "0 0 * * *", "method": "cleanup"}'
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Schedule name |
| `method` | string | Yes | Method to invoke |
| `cron` | string | No | Cron expression (mutually exclusive with `fire_at`) |
| `fire_at` | string | No | One-shot fire time in RFC3339 format |
| `args` | object | No | Method arguments |
| `target_class` | string | No | Target durable object class |
| `target_object_id` | string | No | Target durable object ID |
| `target_orchestration` | string | No | Target orchestration |

#### Get Schedule

```
GET /schedules/{id}
```

```bash
curl http://localhost:18888/schedules/019abc12-...
```

#### Delete Schedule

```
DELETE /schedules/{id}
```

```bash
curl -X DELETE http://localhost:18888/schedules/019abc12-...
```

### Durable Stores

#### List Stores

```
GET /stores
```

```bash
curl http://localhost:18888/stores
```

```json
{
  "success": true,
  "data": [
    {
      "id": "019abc12-...",
      "name": "my-db",
      "kind": "sqlite",
      "sandbox": "my-sandbox",
      "config": {},
      "created_at": "2026-02-18T12:00:00Z",
      "updated_at": "2026-02-18T12:00:00Z"
    }
  ]
}
```

#### Create Store

```
POST /stores
```

```bash
curl -X POST http://localhost:18888/stores \
  -H "Content-Type: application/json" \
  -d '{"name": "my-db", "kind": "sqlite", "sandbox": "my-sandbox"}'
```

**Request body:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Store name |
| `kind` | string | Yes | Engine type: `sqlite`, `kv`, `queue` |
| `sandbox` | string | No | Sandbox to bind to (validated if provided) |
| `config` | object | No | Engine-specific configuration |

#### Get Store

```
GET /stores/{id}
```

```bash
curl http://localhost:18888/stores/019abc12-...
```

#### Delete Store

```
DELETE /stores/{id}
```

```bash
curl -X DELETE http://localhost:18888/stores/019abc12-...
```

#### Query Store

Run a read-only query against a store.

```
POST /stores/{id}/query
```

```bash
curl -X POST http://localhost:18888/stores/019abc12-.../query \
  -H "Content-Type: application/json" \
  -d '{"sql": "SELECT * FROM users WHERE active = ?", "params": [true]}'
```

```json
{
  "success": true,
  "data": {
    "columns": ["id", "name", "active"],
    "rows": [[1, "Alice", true]],
    "row_count": 1
  }
}
```

#### Execute Store

Run a write statement against a store.

```
POST /stores/{id}/execute
```

```bash
curl -X POST http://localhost:18888/stores/019abc12-.../execute \
  -H "Content-Type: application/json" \
  -d '{"sql": "INSERT INTO users (name, active) VALUES (?, ?)", "params": ["Bob", true]}'
```

```json
{
  "success": true,
  "data": {
    "rows_affected": 1
  }
}
```

## Error Responses

```json
{
  "success": false,
  "error": "Sandbox 'missing' not found"
}
```

| Status Code | Meaning |
|-------------|---------|
| 200 | Success |
| 201 | Created |
| 400 | Bad request (validation error) |
| 401 | Unauthorized (missing/invalid API key) |
| 404 | Not found |
| 500 | Internal server error |
