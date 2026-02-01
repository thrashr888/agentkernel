# Nomad Example

Run agentkernel sandboxes as HashiCorp Nomad job allocations.

## Prerequisites

- Nomad cluster or dev agent
- `nomad` CLI
- Docker available on the Nomad client (for the `docker` driver)
- agentkernel built with `--features nomad`

## Quick Start with Nomad Dev

```bash
# Start a local Nomad dev agent
nomad agent -dev &

# Build agentkernel with Nomad support
cargo build --features nomad

# Create and run a sandbox
agentkernel create nomad-sandbox --backend nomad --image alpine:3.20
agentkernel start nomad-sandbox
agentkernel exec nomad-sandbox -- echo "hello from nomad"

# Verify with Nomad CLI
nomad job status agentkernel-nomad-sandbox

# Clean up
agentkernel stop nomad-sandbox
```

## Using the Config File

```bash
agentkernel create nomad-sandbox --config examples/nomad/agentkernel.toml
agentkernel start nomad-sandbox
agentkernel exec nomad-sandbox -- uname -a
agentkernel stop nomad-sandbox
```

## Configuration

See `agentkernel.toml` in this directory. Key settings:

- `[orchestrator].nomad_driver` — Task driver: `docker`, `exec`, or `raw_exec`
- `[orchestrator].nomad_addr` — Nomad API address (or set `NOMAD_ADDR`)
- `[orchestrator].warm_pool_size` — Number of pre-warmed allocations

## Authentication

For production clusters with ACLs:

```bash
export NOMAD_ADDR="http://nomad.example.com:4646"
export NOMAD_TOKEN="s.xxxxxxxxxxxxxxxxxxxxxxxx"
```

## Security

Sandbox jobs run with:
- Docker driver: all capabilities dropped, privileged disabled
- Network isolation via `network_mode: "none"` when `network = false`
- Resource limits enforced (CPU, memory)
