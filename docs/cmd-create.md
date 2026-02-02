
# agentkernel create

Create a new persistent sandbox. The sandbox remains available until explicitly removed.

## Usage

```bash
agentkernel create [OPTIONS] [NAME]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `[NAME]` | Name for the sandbox (alphanumeric, hyphens, underscores). Optional when `--branch` is used. |

## Options

| Option | Description |
|--------|-------------|
| `--config <FILE>` | Path to agentkernel.toml config file |
| `--template <NAME>` | Use a built-in or custom template |
| `--agent <AGENT>` | Agent type: `claude`, `codex`, `gemini`, `opencode` |
| `--dir <PATH>` | Project directory to mount |
| `-B, --backend <BACKEND>` | Backend: `docker`, `podman`, `firecracker`, `apple` |
| `--branch` | Auto-name from git project and branch |
| `--ttl <DURATION>` | Auto-expire after duration (e.g. `1h`, `30m`, `3d`) |

## Examples

### Basic sandbox

```bash
# Create with default settings
agentkernel create my-sandbox

# Create with specific agent preset
agentkernel create my-sandbox --agent claude
```

### Using a config file

```bash
# Create from config (auto-builds Dockerfile if specified)
agentkernel create my-project --config agentkernel.toml

# Use example agent configs
agentkernel create claude-dev --config examples/agents/claude-code/agentkernel.toml
```

### With project directory

```bash
# Mount current directory into sandbox
agentkernel create my-project --config agentkernel.toml --dir .
```

### From a template

```bash
# List available templates
agentkernel template list

# Create from built-in template
agentkernel create my-sandbox --template python
agentkernel create my-sandbox --template rust-ci
agentkernel create my-sandbox --template claude-sandbox
```

### Per-branch sandboxes

```bash
# Auto-derive name from git project + branch (e.g. "myproject-feature-auth")
agentkernel create --branch -B docker

# Reuse the same sandbox across sessions for the same branch
agentkernel create --branch -B docker  # creates or reuses
```

### With TTL (auto-expiry)

```bash
# Sandbox expires after 1 hour
agentkernel create my-sandbox --ttl 1h

# Expires after 3 days
agentkernel create my-sandbox --ttl 3d

# No expiry (default)
agentkernel create my-sandbox --ttl 0
```

Run `agentkernel gc` to garbage-collect expired sandboxes.

### Specify backend

```bash
# Force Docker backend
agentkernel create my-sandbox -B docker

# Use Firecracker (Linux with KVM)
agentkernel create my-sandbox -B firecracker
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
$ agentkernel create my-app --config agentkernel.toml
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

The sandbox is created but not started. Use `agentkernel start` to run it.

## See Also

- [start](../cmd-start-stop) - Start a sandbox
- [Configuration](../config-toml) - Config file format
