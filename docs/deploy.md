
# Deploying agentkernel

Run agentkernel as a service on Kubernetes or Nomad clusters, managing sandboxes via the HTTP API.

## Docker Image

Pre-built images are published to GitHub Container Registry on each release:

```
ghcr.io/thrashr888/agentkernel:latest
ghcr.io/thrashr888/agentkernel:<version>    # e.g. 0.5.0
```

> **Note:** The Docker image is published automatically when a version tag (`v*`) is pushed. If no image exists yet, build locally with `docker build -t agentkernel .` from the repo root.

The Dockerfile builds a release binary with both `kubernetes` and `nomad` feature flags enabled.

## Building from Source

If you prefer to build from source instead of using the Docker image:

```bash
cargo build --release --features kubernetes,nomad
```

Or build only the backend you need:

```bash
cargo build --release --features kubernetes
cargo build --release --features nomad
```

## Backend-Specific Deployment

- **Kubernetes** — Helm chart install, values, RBAC: [Kubernetes Backend](orchestration-kubernetes.md#deploying-agentkernel-on-kubernetes)
- **Nomad** — Job file, Nomad Pack, ACL tokens: [Nomad Backend](orchestration-nomad.md#deploying-agentkernel-on-nomad)

## Environment Variables

| Variable | Backend | Description |
|----------|---------|-------------|
| `KUBECONFIG` | Kubernetes | Path to kubeconfig file |
| `NOMAD_ADDR` | Nomad | Nomad API address |
| `NOMAD_TOKEN` | Nomad | Nomad ACL token |
| `AGENTKERNEL_API_KEY` | Both | API key for HTTP authentication |

## Using the HTTP API

Once deployed, interact with agentkernel via its REST API:

```bash
# Create a sandbox
curl -X POST http://agentkernel:18888/sandboxes \
  -H "Content-Type: application/json" \
  -d '{"name": "my-sandbox", "image": "python:3.12-alpine"}'

# Execute a command
curl -X POST http://agentkernel:18888/sandboxes/my-sandbox/exec \
  -H "Content-Type: application/json" \
  -d '{"command": ["python", "-c", "print(42)"]}'

# Delete the sandbox
curl -X DELETE http://agentkernel:18888/sandboxes/my-sandbox
```

See the [HTTP API Reference](api-http.md) for the full endpoint list.
