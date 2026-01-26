---
title: HTTP API
permalink: /api-http.html
sidebar: agentkernel_sidebar
topnav: topnav
---

# HTTP API

agentkernel includes a REST API for programmatic sandbox management.

## Starting the Server

```bash
# Start on default port (8080)
agentkernel serve

# Custom port
agentkernel serve --port 3000

# With API key authentication
AGENTKERNEL_API_KEY=your-secret agentkernel serve
```

## Authentication

If `AGENTKERNEL_API_KEY` is set, all requests require the `X-API-Key` header:

```bash
curl -H "X-API-Key: your-secret" http://localhost:8080/health
```

## Endpoints

### Health Check

```
GET /health
```

```bash
curl http://localhost:8080/health
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
curl -X POST http://localhost:8080/run \
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

### List Sandboxes

```
GET /sandboxes
```

```bash
curl http://localhost:8080/sandboxes
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
curl -X POST http://localhost:8080/sandboxes \
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
curl http://localhost:8080/sandboxes/my-sandbox
```

### Execute in Sandbox

```
POST /sandboxes/{name}/exec
```

```bash
curl -X POST http://localhost:8080/sandboxes/my-sandbox/exec \
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
curl -X POST http://localhost:8080/sandboxes/my-sandbox/stop
```

### Delete Sandbox

```
DELETE /sandboxes/{name}
```

```bash
curl -X DELETE http://localhost:8080/sandboxes/my-sandbox
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
