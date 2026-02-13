
# agentkernel sandbox list

List all sandboxes and their status.

## Usage

```bash
agentkernel sandbox list [OPTIONS]
```

## Options

| Option | Description |
|--------|-------------|
| `--project` | Filter to sandboxes matching the current git project |

## Output

```
NAME                           STATUS     BACKEND   PORTS
my-project                     running    docker
claude-sandbox                 stopped    docker
web-app                        running    docker    8080:80, 3000:3000
myrepo-feature-auth            running    docker
```

### Columns

| Column | Description |
|--------|-------------|
| NAME | Sandbox name |
| STATUS | `running` or `stopped` |
| BACKEND | Backend used: `docker`, `podman`, `firecracker`, `apple` |
| PORTS | Port mappings (if any) |

## Examples

```bash
# List all sandboxes
agentkernel sandbox list

# Filter to sandboxes for the current git project
# (matches sandboxes whose name starts with the project directory name)
agentkernel sandbox list --project

# No sandboxes
$ agentkernel sandbox list
No sandboxes found.

Create one with: agentkernel sandbox create <name>
```

## Notes

- Lists sandboxes from all backends
- Running status is checked live against the container runtime
- Sandbox state is stored in `~/.local/share/agentkernel/sandboxes/`
- `--project` uses `git rev-parse` to detect the current project name and filters by prefix
