---
layout: default
title: list
parent: Commands
nav_order: 5
---

# agentkernel list

List all sandboxes and their status.

## Usage

```bash
agentkernel list
```

## Output

```
NAME                 STATUS     BACKEND
my-project           running    docker
claude-sandbox       stopped    docker
test-env             running    podman
```

### Columns

| Column | Description |
|--------|-------------|
| NAME | Sandbox name |
| STATUS | `running` or `stopped` |
| BACKEND | Backend used: `docker`, `podman`, `firecracker`, `apple` |

## Examples

```bash
# List all sandboxes
agentkernel list

# No sandboxes
$ agentkernel list
No sandboxes found.

Create one with: agentkernel create <name>
```

## Notes

- Lists sandboxes from all backends
- Running status is checked live against the container runtime
- Sandbox state is stored in `~/.local/share/agentkernel/sandboxes/`
