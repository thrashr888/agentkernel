
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
    {"name": "my-sandbox", "status": "running", "backend": "docker"},
    {"name": "test", "status": "stopped", "backend": "docker"}
  ]
}
```

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

### Get Sandbox

```
GET /sandboxes/{name}
```

```bash
curl http://localhost:18888/sandboxes/my-sandbox
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
