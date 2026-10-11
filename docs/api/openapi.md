
# OpenAPI Specification

The agentkernel HTTP API is documented using OpenAPI 3.1.

## Specification File

Download: [openapi.yaml](https://raw.githubusercontent.com/thrashr888/agentkernel/main/api/openapi.yaml)

## Quick Reference

### Selected endpoints

The downloadable specification is the complete endpoint reference.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check (no auth required) |
| POST | `/run` | Run command in temporary sandbox |
| GET | `/sandboxes` | List all sandboxes |
| POST | `/sandboxes` | Create a sandbox |
| GET | `/sandboxes/{name}` | Get sandbox info |
| GET | `/sandboxes/by-uuid/{uuid}` | Get sandbox info by UUID |
| DELETE | `/sandboxes/{name}` | Remove a sandbox |
| POST | `/sandboxes/{name}/exec` | Execute command in sandbox |
| POST | `/sandboxes/{name}/start` | Start a sandbox |
| POST | `/sandboxes/{name}/stop` | Stop a sandbox |
| POST | `/sandboxes/{name}/pause` | Pause a Firecracker sandbox (preview) |
| POST | `/sandboxes/{name}/resume` | Resume a full-state checkpoint (preview) |
| POST | `/sandboxes/{name}/fork` | Fork a paused Firecracker sandbox (preview) |
| POST | `/sandboxes/{name}/recover` | Recover a quarantined Firecracker sandbox |
| GET | `/orchestrations` | List orchestrations |
| POST | `/orchestrations` | Create orchestration |
| GET | `/orchestrations/definitions` | List orchestration definitions |
| POST | `/orchestrations/definitions` | Create/update orchestration definition |
| GET | `/orchestrations/definitions/{name}` | Get orchestration definition by name |
| DELETE | `/orchestrations/definitions/{name}` | Delete orchestration definition by name |
| GET | `/orchestrations/{id}` | Get orchestration by id |
| POST | `/orchestrations/{id}/events` | Raise external event |
| POST | `/orchestrations/{id}/terminate` | Terminate orchestration |
| GET | `/objects` | List objects |
| POST | `/objects` | Create object |
| GET | `/objects/{id}` | Get object by id |
| GET | `/schedules` | List schedules |
| POST | `/schedules` | Create schedule |
| GET | `/schedules/{id}` | Get schedule by id |
| GET | `/schedules/configured` | List TOML-configured schedules |
| GET | `/schedules/configured/{id}` | Get TOML schedule status |
| GET | `/schedules/configured/{id}/status` | Get TOML schedule status |
| POST | `/schedules/configured/{id}/trigger` | Trigger TOML schedule immediately |
| GET | `/stores` | List durable stores |
| POST | `/stores` | Create durable store |
| GET | `/stores/{id}` | Get durable store by id |
| DELETE | `/stores/{id}` | Delete durable store by id |
| POST | `/stores/{id}/query` | Query durable store |
| POST | `/stores/{id}/execute` | Execute write on durable store |
| POST | `/stores/{id}/command` | Execute command on durable store (Redis) |

### Authentication

Set `AGENTKERNEL_API_KEY` in the server environment to enable authentication. Use the same value in a second terminal for the client examples below.

```bash
# Start server with API key
export AGENTKERNEL_API_KEY="replace-with-your-api-key"
agentkernel serve

# Make authenticated request
curl -H "Authorization: Bearer $AGENTKERNEL_API_KEY" http://localhost:18888/sandboxes
```

### Example: Run Command

```bash
curl -X POST http://localhost:18888/run \
  -H "Authorization: Bearer $AGENTKERNEL_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"command": ["echo", "hello"], "fast": false}'
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
curl -X POST http://localhost:18888/sandboxes \
  -H "Authorization: Bearer $AGENTKERNEL_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name": "my-sandbox", "image": "python:3.12-alpine"}'

# Execute
curl -X POST http://localhost:18888/sandboxes/my-sandbox/exec \
  -H "Authorization: Bearer $AGENTKERNEL_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"command": ["python3", "-c", "print(1+1)"]}'

# Remove
curl -X DELETE http://localhost:18888/sandboxes/my-sandbox \
  -H "Authorization: Bearer $AGENTKERNEL_API_KEY"
```

## Using with API Clients

Import `openapi.yaml` into your favorite API client:

- **Swagger UI**: Paste URL or upload file
- **Postman**: Import the OpenAPI file
- **Insomnia**: Import/Export → Import Data
- **HTTPie**: Use directly with endpoints

## Code Generation

From a repository checkout, generate clients into a separate directory with an OpenAPI 3.1-compatible generator. These examples leave the maintained SDKs untouched:

```bash
# Python client
openapi-generator generate -i api/openapi.yaml -g python -o generated/sdk/python

# TypeScript client
openapi-generator generate -i api/openapi.yaml -g typescript-fetch -o generated/sdk/typescript

# Go client
openapi-generator generate -i api/openapi.yaml -g go -o generated/sdk/go
```
