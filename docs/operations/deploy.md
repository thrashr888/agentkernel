
# Deploying agentkernel

Run agentkernel as a service locally, in the cloud, or on Kubernetes/Nomad clusters. Commands using `examples/` or `deploy/` paths assume a repository checkout.

Starting the HTTP service alone does not provide a sandbox runtime. Verify Docker/Podman access, usable Linux KVM for Firecracker, or a configured remote backend on the deployment host. A dedicated CPU plan does not establish nested-virtualization support.

## Quick Start

| Platform | Difficulty | Time | Best For |
|----------|------------|------|----------|
| [Docker Compose](#docker-compose) | Easy | 2 min | Local dev, small teams |
| [Fly.io](#flyio) | Easy | 5 min | Quick cloud deploy |
| [Railway](#railway) | Easy | 5 min | Prototyping |
| [Hetzner](#hetzner-cloud) | Medium | 15 min | Docker-backed service on a managed host |
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
docker compose up -d

# Check health
curl http://localhost:18888/health
```

With HTTPS via Caddy:

```bash
# Edit Caddyfile with your domain
docker compose --profile with-proxy up -d
```

See [`examples/deploy/docker-compose/`](https://github.com/thrashr888/agentkernel/tree/main/examples/deploy/docker-compose) for full configuration.

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

Check the provider's current pricing and runtime capabilities before deployment. The example does not establish that Firecracker can run inside the selected Fly Machine.

See [`examples/deploy/fly/`](https://github.com/thrashr888/agentkernel/tree/main/examples/deploy/fly) for details.

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

Or use the deploy button: [![Deploy on Railway](https://railway.com/button.svg)](https://railway.com/deploy/v6tIeu?referralCode=gieWq1)

See [`examples/deploy/railway/`](https://github.com/thrashr888/agentkernel/tree/main/examples/deploy/railway) for details.

## Hetzner Cloud

Terraform configuration for a Hetzner Cloud host running the service with Docker. Verify `/dev/kvm` access separately before choosing Firecracker.

```bash
cd examples/deploy/hetzner

export HCLOUD_TOKEN="your-token"
export TF_VAR_ssh_public_key="$(cat ~/.ssh/id_rsa.pub)"
export TF_VAR_api_key="your-agentkernel-api-key"

terraform init
terraform apply
```

Select a server size and region for your workload and confirm current provider pricing.

See [`examples/deploy/hetzner/`](https://github.com/thrashr888/agentkernel/tree/main/examples/deploy/hetzner) for Terraform configuration.

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
- [Kubernetes Backend](kubernetes.md) - Full orchestration docs
- [Enterprise CRDs](kubernetes.md#agentkernelpolicy-crd-enterprise) - AgentKernelPolicy resources

See [`examples/deploy/kubernetes/`](https://github.com/thrashr888/agentkernel/tree/main/examples/deploy/kubernetes) for manifests.

## Nomad

Deploy as a Nomad job:

```bash
nomad job run deploy/nomad/agentkernel.nomad.hcl
```

See [Nomad Backend](nomad.md) for full configuration.

## Building from Source

If you prefer to build from source:

```bash
# Default features include Kubernetes, Nomad, and enterprise policy support
cargo build --release --features kubernetes,nomad

# Or build only a specific optional backend
cargo build --release --no-default-features --features kubernetes
cargo build --release --no-default-features --features nomad
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `AGENTKERNEL_API_KEY` | API key for HTTP authentication |
| `AGENTKERNEL_CONTROL_SOCKET` | Private local CLI/MCP control socket; use the same path on server and clients |
| `KUBECONFIG` | Path to kubeconfig (Kubernetes backend) |
| `NOMAD_ADDR` | Nomad API address |
| `NOMAD_TOKEN` | Nomad ACL token |

Set the listener with `agentkernel serve --host 127.0.0.1 --port 18888`. Configure TLS with `--tls --tls-cert /path/cert.pem --tls-key /path/key.pem`. `AGENTKERNEL_PORT` selects the delegated CLI control port; it does not replace the server's `--port` flag.

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
  -H "Authorization: Bearer $AGENTKERNEL_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"command": ["python", "-c", "print(42)"]}'

# Delete the sandbox
curl -X DELETE http://agentkernel:18888/sandboxes/my-sandbox \
  -H "Authorization: Bearer $AGENTKERNEL_API_KEY"
```

See [HTTP API Reference](../api/http.md) for the full endpoint list.

## Monitoring

All deployments support the `/health` endpoint for health checks:

```bash
curl http://localhost:18888/health
```
