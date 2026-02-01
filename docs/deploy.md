
# Deploying agentkernel

Run agentkernel as a service on Kubernetes or Nomad clusters, managing sandboxes via the HTTP API.

## Prerequisites

Build with orchestration feature flags:

```bash
cargo build --release --features kubernetes,nomad
```

Or build only the backend you need:

```bash
cargo build --release --features kubernetes
cargo build --release --features nomad
```

## Kubernetes (Helm)

### Install

```bash
helm install agentkernel deploy/helm/agentkernel/ \
  --namespace agentkernel-system \
  --create-namespace
```

### Key Configuration

Edit `deploy/helm/agentkernel/values.yaml` or pass `--set` flags:

```yaml
# Backend for sandbox creation
backend: kubernetes

# Orchestrator settings
orchestrator:
  namespace: agentkernel-sandboxes    # Where sandbox pods run
  runtimeClass: ""                     # "gvisor" if available
  warmPoolSize: 10                     # Pre-warmed pods
  maxSandboxes: 200                    # Cluster-wide limit
  serviceAccount: agentkernel-sandbox  # SA for sandbox pods

# Default sandbox resources
sandbox:
  defaults:
    image: alpine:3.20
    memory: 512Mi
    cpu: "1"
    securityProfile: restrictive

# API authentication
apiKey: ""          # Set via --set apiKey=<key> or external secret

# API server resources
resources:
  limits:
    memory: 256Mi
    cpu: 500m
  requests:
    memory: 128Mi
    cpu: 100m

# Autoscaling (disabled by default)
autoscaling:
  enabled: false
  minReplicas: 1
  maxReplicas: 5
```

### What the Chart Creates

| Resource | Purpose |
|----------|---------|
| Deployment | agentkernel API server |
| Service | ClusterIP on port 18888 |
| ServiceAccount | For the API server pod |
| ClusterRole | RBAC for managing sandbox pods |
| ClusterRoleBinding | Binds role to service account |
| ConfigMap | agentkernel.toml configuration |
| Namespace | Sandbox namespace (configurable) |
| Secret | API key (if set) |
| HPA | Horizontal Pod Autoscaler (optional) |

### RBAC

The Helm chart creates a ClusterRole with permissions to:

- Create, delete, list, get pods in the sandbox namespace
- Create and delete NetworkPolicies (for `network: false` sandboxes)
- Exec into pods (for `agentkernel exec`)

### Upgrade and Uninstall

```bash
# Upgrade
helm upgrade agentkernel deploy/helm/agentkernel/ \
  --namespace agentkernel-system

# Uninstall
helm uninstall agentkernel --namespace agentkernel-system
```

## Nomad

### Deploy

```bash
nomad job run deploy/nomad/agentkernel.nomad.hcl
```

### Job Structure

The Nomad job runs agentkernel as a `service` type job with:

- Docker driver with the `ghcr.io/thrashr888/agentkernel:latest` image
- HTTP health check on `/health`
- Port 18888 exposed
- `--backend nomad` flag for sandbox creation

### ACL Token

For production, configure a Nomad ACL token with permissions to submit and manage jobs:

```bash
# Via environment variable
export NOMAD_TOKEN="s.xxxxxxxxxxxxxxxxxxxxxxxx"

# Or via Nomad Variables (recommended for production)
nomad var put nomad/jobs/agentkernel NOMAD_TOKEN="s.xxxx"
```

The job template references the token via the `env` stanza. For Vault integration, use a Vault stanza instead.

### Service Registration

The job registers a `agentkernel` service with Consul/Nomad service discovery and includes an HTTP health check on `/health` every 10 seconds.

### Modify and Redeploy

```bash
# Edit the job file
vim deploy/nomad/agentkernel.nomad.hcl

# Plan changes
nomad job plan deploy/nomad/agentkernel.nomad.hcl

# Apply
nomad job run deploy/nomad/agentkernel.nomad.hcl
```

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
