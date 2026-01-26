
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
| `daemon` | Manage the VM pool daemon |

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
