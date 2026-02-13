
# Nomad Backend

Run sandboxes as HashiCorp Nomad job allocations. Each sandbox is a batch job running `sleep infinity` that accepts commands via `nomad alloc exec`.

## Quick Start

```bash
# Create and run a sandbox on Nomad
agentkernel create my-sandbox --backend nomad --image alpine:3.20
agentkernel start my-sandbox
agentkernel exec my-sandbox -- echo "hello from nomad"
agentkernel stop my-sandbox
```

Or use `run` for ephemeral one-shot execution:

```bash
agentkernel run --backend nomad --image python:3.12-alpine -- python -c "print('hello')"
```

## Configuration

```toml
[orchestrator]
provider = "nomad"
nomad_addr = "http://127.0.0.1:4646"  # Nomad server (default from NOMAD_ADDR)
nomad_driver = "docker"                 # Task driver (default: "docker")
nomad_datacenter = "dc1"               # Nomad datacenter
warm_pool_size = 10                     # Pre-warmed allocations (default: 10)
max_pool_size = 50                      # Maximum total allocations (default: 50)
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `nomad_addr` | string | `NOMAD_ADDR` or `http://127.0.0.1:4646` | Nomad API address |
| `nomad_token` | string | `NOMAD_TOKEN` | ACL token (prefer env var) |
| `nomad_driver` | string | `docker` | Task driver: `docker`, `exec`, `raw_exec` |
| `nomad_datacenter` | string | `dc1` | Target datacenter |
| `warm_pool_size` | int | 10 | Pre-warmed job allocations |
| `max_pool_size` | int | 50 | Maximum concurrent allocations |

## Task Drivers

The Nomad backend supports multiple task drivers:

| Driver | Description | Use Case |
|--------|-------------|----------|
| `docker` | Docker containers (default) | Best isolation, most features |
| `exec` | System isolation | Linux-only, lower overhead |
| `raw_exec` | No isolation | Testing and trusted workloads only |

The `docker` driver provides `cap_drop`, `readonly_rootfs`, and `network_mode` controls. Other drivers use basic resource isolation only.

## Authentication

Set the Nomad ACL token via environment variable (never store in config files):

```bash
export NOMAD_ADDR="http://nomad.example.com:4646"
export NOMAD_TOKEN="s.xxxxxxxxxxxxxxxxxxxxxxxx"
```

## Security

Each sandbox Nomad job runs with:

- Docker driver: `cap_drop = ["ALL"]`, `privileged = false`
- Network isolation: `network_mode = "none"` when `network: false`
- Read-only rootfs option via `readonly_rootfs`
- Resource limits enforced (CPU in MHz, memory in MB)
- Metadata tags: `agentkernel-sandbox={name}`, `agentkernel-managed=true`

## Warm Pool

The Nomad warm pool uses a parameterized batch job (`agentkernel-warm-pool`). Pre-warmed allocations run `sleep infinity` until claimed. When acquired, the dispatched job ID and allocation ID are returned. When released, the allocation is stopped and a replacement dispatched.

A background task runs every 30 seconds to maintain the target warm count.

## Verifying with Nomad CLI

```bash
# List agentkernel jobs
nomad job status

# Check a specific sandbox job
nomad job status agentkernel-my-sandbox

# View allocation details
nomad alloc status <alloc-id>

# View allocation logs
nomad alloc logs <alloc-id>
```

## Deploying agentkernel on Nomad

Run agentkernel itself as a Nomad service that manages sandbox allocations via the HTTP API.

### Deploy with Job File

```bash
# Download the job file
curl -fsSLO https://raw.githubusercontent.com/thrashr888/agentkernel/main/deploy/nomad/agentkernel.nomad.hcl

# Deploy
nomad job run agentkernel.nomad.hcl
```

### Deploy with Nomad Pack

For a configurable deployment using [Nomad Pack](https://github.com/hashicorp/nomad-pack):

```bash
git clone https://github.com/thrashr888/agentkernel.git
nomad-pack run agentkernel/deploy/nomad-pack \
  --var backend=nomad \
  --var count=2
```

See [`deploy/nomad-pack/README.md`](https://github.com/thrashr888/agentkernel/tree/main/deploy/nomad-pack) for all available variables.

### Job Structure

The Nomad job runs agentkernel as a `service` type job with:

- Docker driver with the `ghcr.io/thrashr888/agentkernel:latest` image
- HTTP health check on `/health`
- Port 18888 exposed
- `--backend nomad` flag for sandbox creation

### ACL Token (Production)

Configure a Nomad ACL token with permissions to submit and manage jobs:

```bash
# Via environment variable
export NOMAD_TOKEN="s.xxxxxxxxxxxxxxxxxxxxxxxx"

# Or via Nomad Variables (recommended for production)
nomad var put nomad/jobs/agentkernel NOMAD_TOKEN="s.xxxx"
```

The job template references the token via the `env` stanza. For Vault integration, use a Vault stanza instead.

### Service Registration

The job registers an `agentkernel` service with Consul/Nomad service discovery and includes an HTTP health check on `/health` every 10 seconds.

### Modify and Redeploy

```bash
vim agentkernel.nomad.hcl
nomad job plan agentkernel.nomad.hcl
nomad job run agentkernel.nomad.hcl
```
