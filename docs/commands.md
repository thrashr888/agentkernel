
# Commands

agentkernel provides a Docker-like CLI for managing sandboxes.

## Quick Reference

| Command | Description |
|---------|-------------|
| `run` | Run a command in a temporary sandbox |
| `create` | Create a new sandbox |
| `start` | Start a stopped sandbox |
| `stop` | Stop a running sandbox |
| `remove` | Remove a sandbox |
| `exec` | Execute a command in a running sandbox |
| `attach` | Attach to a sandbox's interactive shell |
| `list` | List all sandboxes |
| `cp` | Copy files to/from a sandbox |
| `setup` | Configure agentkernel and backends |
| `plugin install` | Install agent plugin files (Claude, Codex, Gemini, OpenCode, MCP) |
| `plugin list` | Show available plugins and their install status |
| `agents` | List supported AI agents and their availability |
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

### Persistent sandbox
```bash
agentkernel create my-sandbox
agentkernel start my-sandbox
agentkernel exec my-sandbox -- npm test
agentkernel stop my-sandbox
```

### Interactive development
```bash
agentkernel create dev --config agentkernel.toml
agentkernel start dev
agentkernel attach dev
```

### Session recording and playback
```bash
# Record a session (saves to ~/.agentkernel/recordings/)
agentkernel attach my-sandbox --record

# Replay a recorded session
agentkernel replay ~/.agentkernel/recordings/my-sandbox-20260126-120000.cast

# Replay at 2x speed with max 1s idle time
agentkernel replay session.cast --speed 2.0 --max-idle 1.0
```

### Audit logging
```bash
# List recent audit events
agentkernel audit list

# Show audit entries for a specific sandbox
agentkernel audit list --sandbox my-sandbox

# Show audit log file path
agentkernel audit path
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
