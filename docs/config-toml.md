
# agentkernel.toml

Complete reference for the agentkernel configuration file.

## [sandbox]

Basic sandbox settings.

```toml
[sandbox]
name = "my-project"           # Sandbox name
base_image = "python:3.12"    # Base Docker image (if not using build)
```

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Sandbox name (alphanumeric, hyphens, underscores) |
| `base_image` | string | Docker image to use (ignored if `[build]` is present) |

## [build]

Build a custom Docker image.

```toml
[build]
dockerfile = "Dockerfile"     # Path to Dockerfile (relative to config)
context = "."                 # Build context directory
target = "runtime"            # Multi-stage build target
no_cache = false              # Disable build cache

[build.args]
NODE_VERSION = "22"           # Build arguments
```

| Field | Type | Description |
|-------|------|-------------|
| `dockerfile` | string | Path to Dockerfile |
| `context` | string | Build context (default: Dockerfile's directory) |
| `target` | string | Multi-stage build target |
| `no_cache` | bool | Force rebuild without cache |
| `args` | table | Build arguments passed to `docker build` |

When `dockerfile` is specified, `agentkernel create` automatically builds the image.

## [agent]

AI agent settings.

```toml
[agent]
preferred = "claude"          # Agent type
compatibility_mode = "claude" # Compatibility adjustments
```

| Field | Type | Values |
|-------|------|--------|
| `preferred` | string | `claude`, `codex`, `gemini`, `opencode` |
| `compatibility_mode` | string | Same as preferred |

## [resources]

Resource limits.

```toml
[resources]
vcpus = 2                     # Virtual CPUs
memory_mb = 1024              # Memory in MB
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `vcpus` | int | 1 | Number of virtual CPUs |
| `memory_mb` | int | 512 | Memory limit in megabytes |

## [security]

Security and isolation settings.

```toml
[security]
profile = "moderate"          # Security profile preset
network = true                # Allow network access
mount_cwd = true              # Mount current directory
mount_home = false            # Mount home directory
pass_env = false              # Pass host environment variables
read_only = false             # Read-only root filesystem
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `profile` | string | `moderate` | Preset: `permissive`, `moderate`, `restrictive` |
| `network` | bool | varies | Allow network access |
| `mount_cwd` | bool | varies | Mount current working directory to `/workspace` |
| `mount_home` | bool | varies | Mount `$HOME` to `/home/user` (read-only) |
| `pass_env` | bool | varies | Pass through host environment variables |
| `read_only` | bool | varies | Make root filesystem read-only |

Individual settings override the profile defaults.

## [network]

Advanced network settings.

```toml
[network]
vsock_cid = 3                 # Vsock CID (Firecracker only)
```

## [[files]]

Inject files into the sandbox at startup.

```toml
[[files]]
source = ".env"               # Local file path
dest = "/app/.env"            # Path inside sandbox

[[files]]
source = "config/settings.json"
dest = "/etc/app/settings.json"
```

| Field | Type | Description |
|-------|------|-------------|
| `source` | string | Local file path (relative to config file) |
| `dest` | string | Absolute path inside sandbox |

## Full Example

```toml
[sandbox]
name = "my-fullstack-app"

[build]
dockerfile = "Dockerfile.dev"
context = "."

[build.args]
NODE_VERSION = "22"

[agent]
preferred = "claude"

[resources]
vcpus = 4
memory_mb = 2048

[security]
profile = "moderate"
network = true
mount_cwd = true

[[files]]
source = ".env.development"
dest = "/app/.env"
```
