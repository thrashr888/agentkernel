
# Configuration

agentkernel can be configured via command-line flags or a `agentkernel.toml` config file.

## Config File Location

Place `agentkernel.toml` in your project directory, or specify a path with `--config`:

```bash
# Use config in current directory
agentkernel sandbox create my-sandbox --config agentkernel.toml

# Use config from specific path
agentkernel sandbox create my-sandbox --config /path/to/agentkernel.toml
```

## Quick Example

```toml
[sandbox]
name = "my-project"

[build]
dockerfile = "Dockerfile"

[agent]
preferred = "claude"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"
network = true
mount_cwd = true
```

## Sections

- [agentkernel.toml](toml.md) - Full config file reference
- `[[schedule]]` entries in [agentkernel.toml](toml.md) - daemon-integrated user jobs
- [Security Profiles](security.md) - Permission presets
- [Backends](backends.md) - Backend-specific configuration
