---
title: create
permalink: /cmd-create.html
sidebar: agentkernel_sidebar
topnav: topnav
---

# agentkernel create

Create a new persistent sandbox. The sandbox remains available until explicitly removed.

## Usage

```bash
agentkernel create [OPTIONS] <NAME>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `<NAME>` | Name for the sandbox (alphanumeric, hyphens, underscores) |

## Options

| Option | Description |
|--------|-------------|
| `--config <FILE>` | Path to agentkernel.toml config file |
| `--agent <AGENT>` | Agent type: `claude`, `codex`, `gemini`, `opencode` |
| `--dir <PATH>` | Project directory to mount |
| `--backend <BACKEND>` | Backend: `docker`, `podman`, `firecracker`, `apple` |

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

### Specify backend

```bash
# Force Docker backend
agentkernel create my-sandbox --backend docker

# Use Firecracker (Linux with KVM)
agentkernel create my-sandbox --backend firecracker
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

- [start](start-stop) - Start a sandbox
- [Configuration](../configuration/agentkernel-toml) - Config file format
