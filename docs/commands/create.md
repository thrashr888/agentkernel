
# agentkernel sandbox create

Create a new persistent sandbox. The sandbox remains available until explicitly removed.

## Usage

```bash
agentkernel sandbox create [OPTIONS] [NAME]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `[NAME]` | Name for the sandbox (alphanumeric, hyphens, underscores). Optional when `--branch` is used. |

## Options

| Option | Description |
|--------|-------------|
| `--config <FILE>` | Path to agentkernel.toml config file |
| `--devcontainer <FILE>` | Path to a JSONC Development Container file |
| `--auto-devcontainer` | Detect `.devcontainer/devcontainer.json` in the project |
| `-i, --image <IMAGE>` | Docker image override (takes precedence over the devcontainer) |
| `--template <NAME>` | Use a built-in or custom template |
| `--agent <AGENT>` | Agent type: `claude`, `codex`, `gemini`, `opencode` |
| `--dir <PATH>` | Project directory to mount |
| `--git-worktree` | Create an isolated host-side Git worktree and mount it at `/workspace` |
| `-B, --backend <BACKEND>` | Backend: `docker`, `podman`, `firecracker`, `apple` |
| `--branch` | Auto-name from git project and branch |
| `--ttl <DURATION>` | Auto-expire after duration (e.g. `1h`, `30m`, `3d`) |
| `-p, --publish <PORT>` | Port mapping (e.g. `8080:80`, `3000`, `5353:53/udp`). Repeatable. |
| `--ssh` | Enable SSH access to the sandbox |
| `-S, --secret <BINDING>` | Bind a secret to a host via proxy. Repeatable. See [Secrets](../features/secrets.md). |
| `--secret-file <KEY>` | Inject a vault secret as a file. Repeatable. See [Secrets](../features/secrets.md). |
| `--placeholder-secrets` | Use placeholder tokens for `--secret-file` (real values stay on host). |

## Examples

### Basic sandbox

```bash
# Create with default settings
agentkernel sandbox create my-sandbox

# Create with specific agent preset
agentkernel sandbox create my-sandbox --agent claude
```

### Using a config file

```bash
# Create from config (auto-builds Dockerfile if specified)
agentkernel sandbox create my-project --config agentkernel.toml

# Use example agent configs
agentkernel sandbox create claude-dev --config examples/agents/claude-code/agentkernel.toml
```

### With project directory

```bash
# Mount current directory into sandbox
agentkernel sandbox create my-project --config agentkernel.toml --dir .

# Give the agent a disposable checkout while retaining its branch and commits
agentkernel sandbox create my-project --dir . --git-worktree
```

### From a Development Container file

```bash
agentkernel sandbox create my-project --auto-devcontainer
agentkernel sandbox create my-project --devcontainer .devcontainer/devcontainer.json
```

See [Development Container configuration](../features/devcontainers.md) for
the supported JSONC fields and explicit errors for unsupported features.

### From a template

```bash
# List available templates
agentkernel template list

# Create from built-in template
agentkernel sandbox create my-sandbox --template python
agentkernel sandbox create my-sandbox --template rust-ci
agentkernel sandbox create my-sandbox --template claude-sandbox
```

### Per-branch sandboxes

```bash
# Auto-derive name from git project + branch (e.g. "myproject-feature-auth")
agentkernel sandbox create --branch -B docker

# Reuse the same sandbox across sessions for the same branch
agentkernel sandbox create --branch -B docker  # creates or reuses
```

### With TTL (auto-expiry)

```bash
# Sandbox expires after 1 hour
agentkernel sandbox create my-sandbox --ttl 1h

# Expires after 3 days
agentkernel sandbox create my-sandbox --ttl 3d

# No expiry (default)
agentkernel sandbox create my-sandbox --ttl 0
```

Run `agentkernel sandbox gc` to garbage-collect expired sandboxes.

### Port mapping

```bash
# Map host port 8080 to container port 80
agentkernel sandbox create web-app -p 8080:80

# Multiple port mappings
agentkernel sandbox create web-app -p 8080:80 -p 3000:3000

# Container port only (host port auto-assigned)
agentkernel sandbox create api -p 3000

# UDP port mapping
agentkernel sandbox create dns -p 5353:53/udp
```

Ports are also configurable in `agentkernel.toml`:

```toml
[network]
ports = ["8080:80", "3000"]
```

Git worktree isolation can also be enabled in the project config:

```toml
[git]
# The generated checkout is stored under ~/.local/share/agentkernel/worktrees.
worktree = true
```

### Specify backend

```bash
# Force Docker backend
agentkernel sandbox create my-sandbox -B docker

# Use Firecracker (Linux with KVM)
agentkernel sandbox create my-sandbox -B firecracker
```

## Auto-Build from Dockerfile

When your config specifies a Dockerfile, `create` automatically builds it:

```toml
# agentkernel.toml
[build]
dockerfile = "Dockerfile"

[sandbox]
name = "my-app"
```

```bash
$ agentkernel sandbox create my-app --config agentkernel.toml
Building image from Dockerfile...
Built image: agentkernel-my-app:a1b2c3d4
Creating sandbox 'my-app' with image 'agentkernel-my-app:a1b2c3d4'...
```

Images are cached based on content hash - subsequent creates reuse the cached image.

## What Happens

1. Validates sandbox name
2. Loads config file (if provided)
3. Builds Dockerfile (if configured)
4. Creates container/VM with specified resources
5. Saves sandbox state to `~/.local/share/agentkernel/sandboxes/`

The sandbox is created but not started. Use `agentkernel sandbox start` to run it.

## See Also

- [sandbox start](start-stop.md) - Start a sandbox
- [Configuration](../config/toml.md) - Config file format
