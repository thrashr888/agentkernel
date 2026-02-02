
# Commands

agentkernel provides a Docker-like CLI for managing sandboxes.

## Quick Reference

### Core Sandbox Commands

| Command | Description |
|---------|-------------|
| [`run`](cmd-run) | Run a command in a temporary sandbox |
| [`create`](cmd-create) | Create a new sandbox |
| [`start`](cmd-start-stop) | Start a stopped sandbox |
| [`stop`](cmd-start-stop) | Stop a running sandbox |
| [`remove`](cmd-start-stop) | Remove a sandbox |
| [`exec`](cmd-exec-attach) | Execute a command in a running sandbox |
| [`attach`](cmd-exec-attach) | Attach to a sandbox's interactive shell |
| [`list`](cmd-list) | List all sandboxes |
| `cp` | Copy files to/from a sandbox |
| `info` | Show detailed information about a sandbox |

### Templates & Configuration

| Command | Description |
|---------|-------------|
| [`template list`](cmd-templates) | List available templates (built-in + custom) |
| [`template save`](cmd-templates) | Save a running sandbox as a template |
| [`template add`](cmd-templates) | Add a template from GitHub |
| [`template remove`](cmd-templates) | Remove a custom template |
| [`export-config`](cmd-export-import) | Export sandbox config as TOML |
| [`import-config`](cmd-export-import) | Create sandbox from a TOML config |

### Snapshots & Sessions

| Command | Description |
|---------|-------------|
| [`snapshot take`](cmd-snapshots) | Save a sandbox's current state |
| [`snapshot list`](cmd-snapshots) | List all snapshots |
| [`snapshot delete`](cmd-snapshots) | Delete a snapshot |
| [`snapshot restore`](cmd-snapshots) | Restore a sandbox from a snapshot |
| [`session start`](cmd-sessions) | Start an agent session (sandbox + agent) |
| [`session list`](cmd-sessions) | List all sessions |
| [`session stop`](cmd-sessions) | Stop a session |
| [`session save`](cmd-sessions) | Save a session (snapshot + metadata) |
| [`session resume`](cmd-sessions) | Resume a stopped/saved session |
| [`session delete`](cmd-sessions) | Delete a session |

### Pipelines & Parallel Execution

| Command | Description |
|---------|-------------|
| [`pipeline`](cmd-pipelines) | Run a multi-step pipeline (TOML-defined) |
| [`parallel`](cmd-parallel) | Run multiple jobs concurrently |

### Image & Disk Management

| Command | Description |
|---------|-------------|
| [`images list`](cmd-images) | List Docker images (with sandbox usage) |
| [`images prune`](cmd-images) | Remove unused images |
| [`images pull`](cmd-images) | Pull a Docker image |
| [`export`](cmd-export-import) | Export sandbox filesystem as tar |
| `gc` | Garbage-collect expired sandboxes |
| `clean` | Remove all sandboxes and Docker artifacts |

### Secrets

| Command | Description |
|---------|-------------|
| [`secret set`](cmd-secrets) | Store a secret |
| [`secret get`](cmd-secrets) | Retrieve a secret |
| [`secret list`](cmd-secrets) | List stored secret keys |
| [`secret delete`](cmd-secrets) | Delete a secret |

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
| `policy_violation` | sandbox, policy, details | Blocked command |
