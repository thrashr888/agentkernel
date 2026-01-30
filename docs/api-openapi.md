
# OpenAPI Specification

The agentkernel HTTP API is documented using OpenAPI 3.1.

## Specification File

Download: [openapi.yaml](openapi.yaml)

## Quick Reference

### Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check (no auth required) |
| POST | `/run` | Run command in temporary sandbox |
| GET | `/sandboxes` | List all sandboxes |
| POST | `/sandboxes` | Create a sandbox |
| GET | `/sandboxes/{name}` | Get sandbox info |
| DELETE | `/sandboxes/{name}` | Remove a sandbox |
| POST | `/sandboxes/{name}/exec` | Execute command in sandbox |

### Authentication

Set `AGENTKERNEL_API_KEY` environment variable to enable authentication.

```bash
# Start server with API key
AGENTKERNEL_API_KEY=secret123 agentkernel serve

# Make authenticated request
curl -H "Authorization: Bearer secret123" http://localhost:8880/sandboxes
```

### Example: Run Command

```bash
curl -X POST http://localhost:8880/run \
  -H "Content-Type: application/json" \
  -d '{"command": ["echo", "hello"]}'
```

Response:
```json
{
  "success": true,
  "data": {
    "output": "hello\n"
  }
}
```

### Example: Create and Use Sandbox

```bash
# Create
curl -X POST http://localhost:8880/sandboxes \
  -H "Content-Type: application/json" \
  -d '{"name": "my-sandbox", "image": "python:3.12-alpine"}'

# Execute
curl -X POST http://localhost:8880/sandboxes/my-sandbox/exec \
  -H "Content-Type: application/json" \
  -d '{"command": ["python3", "-c", "print(1+1)"]}'

# Remove
curl -X DELETE http://localhost:8880/sandboxes/my-sandbox
```

## Using with API Clients

Import `openapi.yaml` into your favorite API client:

- **Swagger UI**: Paste URL or upload file
- **Postman**: Import → OpenAPI 3.0
- **Insomnia**: Import/Export → Import Data
- **HTTPie**: Use directly with endpoints

## Code Generation

Generate client SDKs using OpenAPI Generator:

```bash
# Python client
openapi-generator generate -i docs/openapi.yaml -g python -o sdk/python

# TypeScript client
openapi-generator generate -i docs/openapi.yaml -g typescript-fetch -o sdk/typescript

# Go client
openapi-generator generate -i docs/openapi.yaml -g go -o sdk/go
```
