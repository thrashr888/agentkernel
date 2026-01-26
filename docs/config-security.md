---
layout: default
title: Security Profiles
parent: Configuration
nav_order: 2
---

# Security Profiles

agentkernel provides three security profiles that control sandbox permissions.

## Profile Comparison

| Setting | permissive | moderate | restrictive |
|---------|------------|----------|-------------|
| Network access | Yes | Yes | No |
| Mount current directory | Yes | No | No |
| Mount home directory | Yes | No | No |
| Pass environment variables | Yes | No | No |
| Read-only filesystem | No | No | Yes |

## permissive

Maximum access for trusted workloads. Use when you need full integration with your development environment.

```toml
[security]
profile = "permissive"
```

Equivalent to:
```toml
[security]
network = true
mount_cwd = true
mount_home = true
pass_env = true
read_only = false
```

**Use cases:**
- Local development with file watching
- Running your own trusted code
- Integration testing

## moderate

Balanced security for most use cases. Default profile.

```toml
[security]
profile = "moderate"
```

Equivalent to:
```toml
[security]
network = true
mount_cwd = false
mount_home = false
pass_env = false
read_only = false
```

**Use cases:**
- Running AI agents
- Executing untrusted code with network needs
- Most development workflows

## restrictive

Maximum isolation for untrusted code.

```toml
[security]
profile = "restrictive"
```

Equivalent to:
```toml
[security]
network = false
mount_cwd = false
mount_home = false
pass_env = false
read_only = true
```

**Use cases:**
- Running completely untrusted code
- Security-sensitive environments
- Compliance requirements

## Overriding Profile Settings

You can start with a profile and override specific settings:

```toml
[security]
profile = "moderate"
mount_cwd = true    # Override: enable mounting current directory
```

## Command-Line Override

```bash
# Use restrictive profile for a run command
agentkernel run --profile restrictive python3 untrusted_script.py
```

## Environment Variable Passthrough

When `pass_env = true` (permissive profile), these environment variables are passed through:

- `PATH`
- `HOME`
- `USER`
- `LANG`
- `LC_ALL`
- `TERM`

For API keys and secrets, use the `-e` flag explicitly:

```bash
agentkernel exec my-sandbox -e API_KEY=$API_KEY -- ./script.sh
```

This is more secure than `pass_env = true` because you control exactly which variables are passed.
