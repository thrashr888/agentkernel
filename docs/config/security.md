
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

### permissive

Full access for trusted workloads — local development, your own code, integration testing.

```toml
[security]
profile = "permissive"
```

### moderate (default)

Balanced security — AI agents, untrusted code with network needs, most workflows.

```toml
[security]
profile = "moderate"
```

### restrictive

Maximum isolation — completely untrusted code, compliance requirements.

```toml
[security]
profile = "restrictive"
```

### Overriding Settings

Start with a profile and override individual settings:

```toml
[security]
profile = "moderate"
mount_cwd = true    # Override: enable mounting current directory
```

```bash
agentkernel run --profile restrictive python3 untrusted_script.py
```

## Environment Variable Passthrough

When `pass_env = true` (permissive profile), these variables are passed through: `PATH`, `HOME`, `USER`, `LANG`, `LC_ALL`, `TERM`.

For API keys, use the [secrets system](../features/secrets.md) instead of environment variables:

```bash
agentkernel sandbox create my-agent --secret OPENAI_API_KEY:api.openai.com          # best
agentkernel sandbox create my-agent --secret-file OPENAI_API_KEY --placeholder-secrets  # good
agentkernel sandbox create my-agent --secret-file OPENAI_API_KEY                     # acceptable
```

Environment variables are visible to all processes and appear in `/proc/*/environ`.

## Domain Filtering

> **Not yet enforced at runtime.** Rules are parsed and validated but DNS enforcement requires the Firecracker backend. A warning is printed at startup when domain rules are configured.

```toml
[security.domains]
allow = ["api.anthropic.com", "*.github.com", "pypi.org"]
allowlist_only = true

# Or blocklist mode
block = ["evil.com", "*.malware.net"]
allowlist_only = false  # default
```

| Setting | Description |
|---------|-------------|
| `allow` | Allowed domains (supports `*.domain.com` wildcards) |
| `block` | Blocked domains (supports `*.domain.com` wildcards) |
| `allowlist_only` | If true, only `allow` list domains are permitted |

## Command Filtering

Command filtering is **enforced at runtime**. Blocked commands are rejected and logged as `PolicyViolation` audit events.

```toml
[security.commands]
allow = ["python3", "pip", "git", "node", "npm"]
allowlist_only = true

# Or blocklist mode
block = ["curl", "wget", "nc", "ncat", "ssh", "scp"]
allowlist_only = false  # default
```

## Seccomp Profiles

Restrict which system calls a process can make.

```toml
[security]
seccomp = "moderate"  # or path to custom profile JSON
```

| Profile | Description |
|---------|-------------|
| `default` | Allow most syscalls, block dangerous ones (mount, reboot) |
| `moderate` | Block dangerous syscalls + ptrace |
| `restrictive` | Allowlist-only with minimal syscalls |
| `ai-agent` | Optimized for AI coding agents (file/network/process) |

Each security profile automatically uses a matching seccomp profile. AI agent compatibility modes (Claude, Codex, Gemini) use `ai-agent`.

Custom profiles follow the [Docker seccomp profile format](https://docs.docker.com/engine/security/seccomp/). Built-in profiles are searched in `./images/seccomp/`, `<executable-dir>/seccomp/`, and `/usr/share/agentkernel/seccomp/`.

## Config Validation

agentkernel validates security config at startup and warns about:

- Domain rules with network disabled (no effect)
- Conflicting domain lists (block takes precedence)
- Unenforceable domain rules (runtime enforcement not yet available)
