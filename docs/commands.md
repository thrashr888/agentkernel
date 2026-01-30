
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
