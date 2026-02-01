# agentkernel Nomad Pack

Deploy agentkernel as a Nomad service for sandboxed AI agent execution.

## Prerequisites

- [Nomad](https://www.nomadproject.io/) cluster
- [nomad-pack](https://github.com/hashicorp/nomad-pack) CLI
- Docker driver enabled on Nomad clients

## Usage

```bash
# Render and deploy with defaults
nomad-pack run deploy/nomad-pack

# Override variables
nomad-pack run deploy/nomad-pack \
  --var count=3 \
  --var backend=nomad \
  --var nomad_addr=http://localhost:4646

# Destroy the deployment
nomad-pack destroy agentkernel
```

## Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `job_name` | `agentkernel` | Nomad job name |
| `datacenters` | `["dc1"]` | Eligible datacenters |
| `region` | `""` | Nomad region |
| `image` | `ghcr.io/thrashr888/agentkernel:latest` | Container image |
| `image_tag` | `""` | Override image tag |
| `count` | `1` | Number of instances |
| `http_port` | `18888` | HTTP listen port |
| `backend` | `nomad` | Sandbox backend (`nomad`, `docker`, `kubernetes`) |
| `nomad_addr` | `""` | Nomad API address for sandbox backend |
| `nomad_token` | `""` | Nomad ACL token (prefer Vault for production) |
| `resources.cpu` | `500` | CPU allocation (MHz) |
| `resources.memory` | `256` | Memory allocation (MB) |
| `register_consul_service` | `true` | Register as Consul service |

## Health Check

The pack registers a Consul health check at `/health` on the HTTP port.
