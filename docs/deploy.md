
# Deploying agentkernel

Run agentkernel as a service locally, in the cloud, or on Kubernetes/Nomad clusters.

## Quick Start

| Platform | Difficulty | Time | Best For |
|----------|------------|------|----------|
| [Docker Compose](#docker-compose) | Easy | 2 min | Local dev, small teams |
| [Fly.io](#flyio) | Easy | 5 min | Quick cloud deploy |
| [Railway](#railway) | Easy | 5 min | Prototyping |
| [Hetzner](#hetzner-cloud) | Medium | 15 min | Production, Firecracker |
| [Kubernetes](#kubernetes) | Complex | 30 min | Enterprise |
| [Nomad](#nomad) | Medium | 15 min | HashiCorp shops |

## Docker Image

Pre-built images are published to GitHub Container Registry:

```
ghcr.io/thrashr888/agentkernel:latest
ghcr.io/thrashr888/agentkernel:<version>    # e.g. 0.8.0
```

## Docker Compose

Local multi-container setup for development and small teams.

```bash
cd examples/deploy/docker-compose
docker-compose up -d

# Check health
curl http://localhost:18888/health
```

With HTTPS via Caddy:

```bash
# Edit Caddyfile with your domain
docker-compose --profile with-proxy up -d
```

See [`examples/deploy/docker-compose/`](../examples/deploy/docker-compose/) for full configuration.

## Fly.io

One-click deployment with persistent storage.

```bash
cd examples/deploy/fly

# First time
fly launch --copy-config

# Updates
fly deploy

# Set secrets
fly secrets set AGENTKERNEL_API_KEY=your-key
```

Costs: ~$5-10/month (shared CPU), ~$30-60/month (dedicated CPU for Firecracker).

See [`examples/deploy/fly/`](../examples/deploy/fly/) for details.

## Railway

Simple deployment for prototyping.

```bash
cd examples/deploy/railway

# Install CLI
npm install -g @railway/cli

# Deploy
railway login
railway init
railway up
```

See [`examples/deploy/railway/`](../examples/deploy/railway/) for details.

## Hetzner Cloud

Bare metal-like performance at low cost. Best for production Firecracker deployments.

```bash
cd examples/deploy/hetzner

export HCLOUD_TOKEN="your-token"
export TF_VAR_ssh_public_key="$(cat ~/.ssh/id_rsa.pub)"
export TF_VAR_api_key="your-agentkernel-api-key"

terraform init
terraform apply
```

Costs: €4.49-65.99/month depending on server size.

See [`examples/deploy/hetzner/`](../examples/deploy/hetzner/) for Terraform configuration.

## Kubernetes

Deploy with Kustomize:

```bash
cd examples/deploy/kubernetes

# Deploy
kubectl apply -k kustomize/

# Check status
kubectl -n agentkernel get pods

# Port forward
kubectl -n agentkernel port-forward svc/agentkernel 18888:18888
```

For advanced deployments, see:
- [Kubernetes Backend](orchestration-kubernetes.md) - Full orchestration docs
- [Enterprise CRDs](enterprise.md) - AgentKernelPolicy resources

See [`examples/deploy/kubernetes/`](../examples/deploy/kubernetes/) for manifests.

## Nomad

Deploy as a Nomad job:

```bash
nomad job run agentkernel.nomad
```

See [Nomad Backend](orchestration-nomad.md) for full configuration.

## Building from Source

If you prefer to build from source:

```bash
# All features
cargo build --release --features kubernetes,nomad

# Or specific backend
cargo build --release --features kubernetes
cargo build --release --features nomad
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `AGENTKERNEL_API_KEY` | API key for HTTP authentication |
| `AGENTKERNEL_HOST` | Bind address (default: 127.0.0.1) |
| `AGENTKERNEL_PORT` | Listen port (default: 18888) |
| `AGENTKERNEL_TLS_CERT` | Path to TLS certificate |
| `AGENTKERNEL_TLS_KEY` | Path to TLS private key |
| `KUBECONFIG` | Path to kubeconfig (Kubernetes backend) |
| `NOMAD_ADDR` | Nomad API address |
| `NOMAD_TOKEN` | Nomad ACL token |

## Using the HTTP API

Once deployed, interact via REST API:

```bash
# Create a sandbox
curl -X POST http://agentkernel:18888/sandboxes \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $AGENTKERNEL_API_KEY" \
  -d '{"name": "my-sandbox", "image": "python:3.12-alpine"}'

# Execute a command
curl -X POST http://agentkernel:18888/sandboxes/my-sandbox/exec \
  -H "Content-Type: application/json" \
  -d '{"command": ["python", "-c", "print(42)"]}'

# Delete the sandbox
curl -X DELETE http://agentkernel:18888/sandboxes/my-sandbox
```

See [HTTP API Reference](api-http.md) for the full endpoint list.

## Monitoring

All deployments support the `/health` endpoint for health checks:

```bash
curl http://localhost:18888/health
```

For metrics and observability, see [Telemetry](telemetry.md) (coming soon).
