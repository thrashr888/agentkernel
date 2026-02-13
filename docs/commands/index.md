
# Commands

agentkernel provides a Docker-like CLI for managing sandboxes.

## Quick Reference

### Core Sandbox Commands

| Command | Description |
|---------|-------------|
| [`run`](run.md) | Run a command in a temporary sandbox |
| [`create`](create.md) | Create a new sandbox |
| [`start`](start-stop.md) | Start a stopped sandbox |
| [`stop`](start-stop.md) | Stop a running sandbox |
| [`remove`](start-stop.md) | Remove a sandbox |
| [`exec`](exec-attach.md) | Execute a command in a running sandbox |
| [`attach`](exec-attach.md) | Attach to a sandbox's interactive shell |
| `ssh` | SSH into a sandbox (certificate-authenticated) |
| `ssh-config` | Generate SSH config entry for IDE integration |
| `ssh-proxy` | ProxyCommand helper for SSH |
| [`list`](list.md) | List all sandboxes (with IP addresses) |
| `extend-ttl` | Extend a sandbox's time-to-live |
| `cp` | Copy files to/from a sandbox |
| `info` | Show detailed information about a sandbox (with IP) |

### Templates & Configuration

| Command | Description |
|---------|-------------|
| [`template list`](templates.md) | List available templates (built-in + custom) |
| [`template save`](templates.md) | Save a running sandbox as a template |
| [`template add`](templates.md) | Add a template from GitHub |
| [`template remove`](templates.md) | Remove a custom template |
| [`export-config`](export-import.md) | Export sandbox config as TOML |
| [`import-config`](export-import.md) | Create sandbox from a TOML config |

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
| [`export`](export-import.md) | Export sandbox filesystem as tar |
| `gc` | Garbage-collect expired sandboxes |
| `clean` | Remove all sandboxes and Docker artifacts |

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

## Global Options

```
--help, -h      Show help
--version, -V   Show version
```

## Common Workflows

### One-shot execution
```bash
agentkernel run python3 script.py
```

### Create from template
```bash
agentkernel create my-sandbox --template python -B docker
agentkernel start my-sandbox
agentkernel exec my-sandbox -- python3 --version
```

### Per-branch sandboxes
```bash
# Auto-names sandbox from git project + branch
agentkernel create --branch -B docker
agentkernel list --project    # Filter to current project
```

### Persistent sandbox
```bash
agentkernel create my-sandbox
agentkernel start my-sandbox
agentkernel exec my-sandbox -- npm test
agentkernel stop my-sandbox
```

### Sandbox with TTL
```bash
# Create a sandbox with 2-hour TTL
agentkernel create my-sandbox --ttl 2h
agentkernel start my-sandbox

# Extend the TTL by 1 hour (default)
agentkernel extend-ttl my-sandbox

# Extend by specific duration
agentkernel extend-ttl my-sandbox --by 30m
```

### Snapshot and restore
```bash
agentkernel snapshot take my-sandbox --name my-checkpoint
agentkernel snapshot restore my-checkpoint --as restored-sandbox
```

### Agent sessions
```bash
agentkernel session start --name feature-x --agent claude -B docker
agentkernel exec session-feature-x -- echo "working"
agentkernel session save feature-x
agentkernel session resume feature-x
```

### Parallel execution
```bash
agentkernel parallel \
  --job "lint:node:22-alpine:npx eslint ." \
  --job "test:node:22-alpine:npm test" \
  -B docker
```

### Interactive development
```bash
agentkernel create dev --config agentkernel.toml
agentkernel start dev
agentkernel attach dev
```

### Persistent volumes
```bash
# Create a volume
agentkernel volume create mydata

# Use it in a sandbox
agentkernel create dev --volume mydata:/data
agentkernel start dev
agentkernel exec dev -- echo "hello" > /data/test.txt
agentkernel stop dev

# Data persists across restarts
agentkernel start dev
agentkernel exec dev -- cat /data/test.txt
```

### Custom images
```bash
# Build from Dockerfile
agentkernel build -t my-tools .

# Use in a sandbox
agentkernel create dev --image my-tools
```

### SSH access
```bash
# Create with SSH enabled
agentkernel create dev --ssh -B docker
agentkernel start dev

# SSH in (generates ephemeral cert automatically)
agentkernel ssh dev

# Run a command over SSH
agentkernel ssh dev -- ls -la /

# Record the session
agentkernel ssh dev --record ./session.cast

# Generate SSH config for VS Code Remote-SSH
agentkernel ssh-config dev >> ~/.ssh/config
```

### Session recording and playback
```bash
# Record a session
agentkernel attach my-sandbox --record session.cast

# Replay a recorded session
agentkernel replay ~/.agentkernel/recordings/my-sandbox-20260126-120000.cast

# Replay at 2x speed with max 1s idle time
agentkernel replay session.cast --speed 2.0 --max-idle 1.0
```

### Audit logging
```bash
# List recent audit events
agentkernel audit

# Show audit entries for a specific sandbox
agentkernel audit --sandbox my-sandbox

# Show audit log file path
agentkernel audit --path
```

The audit log is stored as JSONL at `~/.agentkernel/audit.jsonl`. Each line is a JSON object with `timestamp`, `pid`, `user`, and the event payload. Set `AGENTKERNEL_AUDIT=0` to disable.

**Event types:**

| Event | Fields | When |
|-------|--------|------|
| `sandbox_created` | name, image, backend | `create` |
| `sandbox_started` | name, profile | `start` |
| `sandbox_stopped` | name | `stop` |
| `sandbox_removed` | name | `remove` |
| `command_executed` | sandbox, command, exit_code | `exec` / `run` |
| `file_written` | sandbox, path | `cp` to sandbox |
| `file_read` | sandbox, path | `cp` from sandbox |
| `session_attached` | sandbox | `attach` |
| `ssh_connected` | sandbox, user, cert_fingerprint | `ssh` (connect) |
| `ssh_disconnected` | sandbox, duration_secs, recording | `ssh` (disconnect) |
| `policy_violation` | sandbox, policy, details | Blocked command |
