
# Commands

agentkernel provides a Docker-like CLI for managing sandboxes.

## Quick Reference

### Daily Drivers (Root Level)

| Command | Description |
|---------|-------------|
| [`run`](run.md) | Run a command in a temporary sandbox |
| [`exec`](exec-attach.md) | Execute a command in a running sandbox |
| [`attach`](exec-attach.md) | Attach to a sandbox's interactive shell |
| [`receipt`](receipts.md) | Verify and replay execution receipts |

### Sandbox Lifecycle (`sandbox` / `sb`)

| Command | Description |
|---------|-------------|
| [`sandbox create`](create.md) | Create a new sandbox |
| [`sandbox start`](start-stop.md) | Start a stopped sandbox |
| [`sandbox stop`](start-stop.md) | Stop a running sandbox |
| [`sandbox pause`](start-stop.md) | Checkpoint a Firecracker sandbox with guest memory and processes (`suspend` alias) |
| [`sandbox resume`](start-stop.md) | Resume a full-state Firecracker checkpoint |
| [`sandbox fork`](start-stop.md) | Start a new sandbox from a paused Firecracker checkpoint |
| [`sandbox remove`](start-stop.md) | Remove a sandbox |
| [`sandbox list`](list.md) | List all sandboxes (with IP addresses) |
| `sandbox info` | Show detailed information about a sandbox (with IP) |
| `sandbox extend-ttl` | Extend a sandbox's time-to-live |
| `sandbox cp` | Copy files to/from a sandbox |
| `sandbox gc` | Garbage-collect expired sandboxes |
| `sandbox clean` | Remove all sandboxes and Docker artifacts |
| `sandbox exec-list` | List running exec processes |
| `sandbox exec-logs` | View exec process logs |
| `sandbox exec-kill` | Kill a running exec process |

### SSH (`ssh`)

| Command | Description |
|---------|-------------|
| `ssh connect` | SSH into a sandbox (certificate-authenticated) |
| `ssh config` | Generate SSH config entry for IDE integration |
| `ssh proxy` | ProxyCommand helper for SSH |

### Templates & Configuration

| Command | Description |
|---------|-------------|
| [`template list`](templates.md) | List available templates (built-in + custom) |
| [`template save`](templates.md) | Save a running sandbox as a template |
| [`template add`](templates.md) | Add a template from GitHub |
| [`template remove`](templates.md) | Remove a custom template |
| [`sandbox export-config`](export-import.md) | Export sandbox config as TOML |
| [`sandbox import-config`](export-import.md) | Create sandbox from a TOML config |

### Snapshots & Sessions

| Command | Description |
|---------|-------------|
| [`snapshot take`](snapshots.md) | Save a sandbox's current state |
| [`snapshot list`](snapshots.md) | List all snapshots |
| [`snapshot delete`](snapshots.md) | Delete a snapshot |
| [`snapshot restore`](snapshots.md) | Restore a sandbox from a snapshot |
| [`session start`](sessions.md) | Start an agent session (sandbox + agent) |
| [`session list`](sessions.md) | List all sessions |
| [`session stop`](sessions.md) | Stop a session |
| [`session save`](sessions.md) | Save a session (snapshot + metadata) |
| [`session resume`](sessions.md) | Resume a stopped/saved session |
| [`session delete`](sessions.md) | Delete a session |

### Pipelines & Parallel Execution

| Command | Description |
|---------|-------------|
| [`pipeline`](pipelines.md) | Run a multi-step pipeline (TOML-defined) |
| [`parallel`](parallel.md) | Run multiple jobs concurrently |
| [`task run`](tasks.md) | Drain durable agent tasks with bounded concurrency |

### Volumes

| Command | Description |
|---------|-------------|
| [`volume create`](volumes.md) | Create a persistent volume |
| [`volume list`](volumes.md) | List all volumes |
| [`volume info`](volumes.md) | Show volume details |
| [`volume delete`](volumes.md) | Delete a volume |

### Image & Disk Management

| Command | Description |
|---------|-------------|
| [`build`](images.md) | Build a custom image from Dockerfile |
| [`images list`](images.md) | List Docker images (with sandbox usage) |
| [`images local-list`](images.md) | List locally built images |
| [`images local-delete`](images.md) | Delete a locally built image |
| [`images local-sync`](images.md) | Sync metadata with Docker |
| [`images prune`](images.md) | Remove unused images |
| [`images pull`](images.md) | Pull a Docker image |
| [`sandbox export`](export-import.md) | Export sandbox filesystem as tar |

### Secrets

| Command | Description |
|---------|-------------|
| [`secret set`](secrets.md) | Store a secret |
| [`secret get`](secrets.md) | Retrieve a secret |
| [`secret list`](secrets.md) | List stored secret keys |
| [`secret delete`](secrets.md) | Delete a secret |

### System & Diagnostics

| Command | Description |
|---------|-------------|
| `setup` | Configure agentkernel and backends |
| `doctor` | System diagnostics and health check |
| `stats` | Show usage statistics from audit log |
| `benchmark` | Benchmark sandbox backends |
| `completions` | Generate shell completions (bash, zsh, fish) |
| `agents` | List supported AI agents and availability |
| `plugin` | Manage agent plugins |
| `daemon` | Manage the VM pool daemon |
| `audit` | View and manage audit logs |
| `replay` | Replay a recorded session |
| [`receipt verify`](receipts.md) | Verify execution receipt integrity |
| [`receipt replay`](receipts.md) | Replay a recorded command invocation |

## Global Options

```
--help, -h      Show help
--version, -V   Show version
```

## Common Workflows

```bash
# One-shot execution
agentkernel run python3 script.py

# Persistent sandbox
agentkernel sandbox create my-sandbox --template python -B docker
agentkernel sandbox start my-sandbox
agentkernel exec my-sandbox -- python3 --version
agentkernel sandbox stop my-sandbox

# Per-branch sandboxes (auto-named from git project + branch)
agentkernel sandbox create --branch -B docker

# Interactive development
agentkernel sandbox create dev --config agentkernel.toml
agentkernel sandbox start dev
agentkernel attach dev
```

See individual command pages for detailed examples: [run](run.md), [create](create.md), [snapshots](snapshots.md), [sessions](sessions.md), [pipelines](pipelines.md), [volumes](volumes.md), [images](images.md), [receipts](receipts.md).

## Audit Logging

All sandbox operations are logged to `~/.agentkernel/audit.jsonl` as JSONL. Each entry includes `timestamp`, `pid`, `user`, and the event payload. Set `AGENTKERNEL_AUDIT=0` to disable.

```bash
agentkernel audit                          # list recent events
agentkernel audit --sandbox my-sandbox     # filter by sandbox
agentkernel audit --path                   # show log file path
```

| Event | When |
|-------|------|
| `sandbox_created` | `sandbox create` |
| `sandbox_started` | `sandbox start` |
| `sandbox_stopped` | `sandbox stop` |
| `sandbox_removed` | `sandbox remove` |
| `command_executed` | `exec` / `run` |
| `file_written` | `sandbox cp` to sandbox |
| `file_read` | `sandbox cp` from sandbox |
| `session_attached` | `attach` |
| `ssh_connected` | `ssh connect` |
| `ssh_disconnected` | `ssh connect` (disconnect) |
| `policy_violation` | Blocked command |
