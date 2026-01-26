---
layout: default
title: Configuration
nav_order: 5
has_children: true
---

# Configuration

agentkernel can be configured via command-line flags or a `agentkernel.toml` config file.

## Config File Location

Place `agentkernel.toml` in your project directory, or specify a path with `--config`:

```bash
# Use config in current directory
agentkernel create my-sandbox --config agentkernel.toml

# Use config from specific path
agentkernel create my-sandbox --config /path/to/agentkernel.toml
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

- [agentkernel.toml](agentkernel-toml) - Full config file reference
- [Security Profiles](security-profiles) - Permission presets
- [Backends](backends) - Backend-specific configuration
