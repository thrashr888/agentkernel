
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

## Domain Filtering

Control which network domains the sandbox can access.

```toml
[security.domains]
# Only allow specific domains (allowlist mode)
allow = ["api.anthropic.com", "*.github.com", "pypi.org"]
allowlist_only = true

# Or block specific domains (blocklist mode)
block = ["evil.com", "*.malware.net"]
allowlist_only = false  # default
```

| Setting | Description |
|---------|-------------|
| `allow` | List of allowed domains (supports `*.domain.com` wildcards) |
| `block` | List of blocked domains (supports `*.domain.com` wildcards) |
| `allowlist_only` | If true, only domains in `allow` list are permitted |

**Examples:**

```toml
# Allow only AI API endpoints
[security.domains]
allow = ["api.anthropic.com", "api.openai.com", "generativelanguage.googleapis.com"]
allowlist_only = true

# Block known malicious domains
[security.domains]
block = ["*.ru", "*.cn", "malware-c2.com"]
```

## Command Filtering

Control which commands and binaries can be executed inside the sandbox.

```toml
[security.commands]
# Only allow specific commands (allowlist mode)
allow = ["python3", "pip", "git", "node", "npm"]
allowlist_only = true

# Or block specific commands (blocklist mode)
block = ["curl", "wget", "nc", "ncat"]
allowlist_only = false  # default
```

| Setting | Description |
|---------|-------------|
| `allow` | List of allowed commands/binaries |
| `block` | List of blocked commands/binaries |
| `allowlist_only` | If true, only commands in `allow` list can be executed |

**Examples:**

```toml
# Minimal Python environment
[security.commands]
allow = ["python3", "pip", "git"]
allowlist_only = true

# Block network tools
[security.commands]
block = ["curl", "wget", "nc", "ncat", "ssh", "scp"]
```

## Seccomp Profiles

Apply seccomp profiles for syscall filtering. Seccomp (Secure Computing Mode) restricts which system calls a process can make, providing an additional layer of defense.

```toml
[security]
seccomp = "moderate"  # or path to custom profile
```

### Built-in Profiles

| Profile | Description | Use Case |
|---------|-------------|----------|
| `default` | Allow most syscalls, block dangerous ones (mount, reboot, etc.) | Permissive environments |
| `moderate` | Block dangerous syscalls + ptrace | Balanced security (default for moderate profile) |
| `restrictive` | Allowlist-only with minimal syscalls | High-security environments |
| `ai-agent` | Optimized for AI coding agents with file/network/process syscalls | Claude, Codex, Gemini agents |

### Profile Mapping

Each security profile automatically uses an appropriate seccomp profile:

| Security Profile | Default Seccomp |
|-----------------|-----------------|
| `permissive` | `default` |
| `moderate` | `moderate` |
| `restrictive` | `restrictive` |

AI agent compatibility modes (Claude, Codex, Gemini) use the `ai-agent` profile.

### Custom Profiles

You can provide a path to a custom seccomp profile JSON file:

```toml
[security]
seccomp = "/path/to/custom-seccomp.json"
```

Custom profiles should follow the [Docker seccomp profile format](https://docs.docker.com/engine/security/seccomp/).

### Profile Locations

Built-in profiles are searched in the following locations:

1. `./images/seccomp/` (development)
2. `<executable-dir>/seccomp/` (installed)
3. `/usr/share/agentkernel/seccomp/` (system)
