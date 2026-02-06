
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
AGENTKERNEL_API_KEY=your-secret agentkernel serve
```

## Authentication

If `AGENTKERNEL_API_KEY` is set, all requests require the `X-API-Key` header:

```bash
curl -H "X-API-Key: your-secret" http://localhost:18888/health
```

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
```

```bash
curl http://localhost:18888/sandboxes
```

```json
{
  "success": true,
  "data": [
    {"name": "my-sandbox", "status": "running", "backend": "docker", "ip": "172.17.0.3"},
    {"name": "test", "status": "stopped", "backend": "docker"}
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
  "data": {"name": "my-sandbox", "status": "running", "backend": "docker"}
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

**With resource limits:**

```bash
curl -X POST http://localhost:18888/sandboxes \
  -H "Content-Type: application/json" \
  -d '{"name": "big", "vcpus": 2, "memory_mb": 1024, "profile": "restrictive"}'
```

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

Returns all audit events associated with the sandbox, sorted by timestamp. See [audit logging](../commands#audit-logging) for event types.

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
