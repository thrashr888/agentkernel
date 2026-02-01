# OSS CLI Roadmap

Local-first features for agentkernel as an open-source CLI tool. No remote host required.

## Already Shipped (v0.5.1)

The original SaaSify plan proposed auth, multi-tenancy, billing, and daemon integration. Here's what actually shipped:

- [x] HTTP API — 12+ endpoints, SSE streaming, batch execution, OpenAPI 3.1
- [x] API key auth — `AGENTKERNEL_API_KEY` env var or config
- [x] Daemon mode — warm VM pools, ~195ms latency, per-runtime pools
- [x] Daemon ↔ HTTP integration — daemon auto-used by `serve`
- [x] Audit logging — JSONL format, per-sandbox filtering, event types
- [x] Session recording — asciicast v2, replay with speed control
- [x] 7 backends — Docker, Podman, Firecracker, Apple Containers, Hyperlight, Kubernetes, Nomad
- [x] Security profiles — permissive/moderate/restrictive, domain filtering, command filtering, seccomp
- [x] MCP server — JSON-RPC 2.0 over stdio, 9 tools
- [x] Plugin system — Claude, Codex, Gemini, OpenCode installers
- [x] 5 SDKs — Node.js, Python, Rust, Go, Swift
- [x] Language detection — 12+ runtimes auto-detected from project files
- [x] Dockerfile support — auto-build with caching, multi-stage, build args
- [x] File injection — `[[files]]` config, read/write/delete API
- [x] Resource limits — vCPUs, memory_mb per sandbox
- [x] Orchestration — Kubernetes (CRDs, operator, Helm), Nomad (warm pools, Nomad Pack)

## P0: Quick Wins

### `doctor` command

System diagnostics that go beyond `agentkernel status`.

```
$ agentkernel doctor

Backend Health:
  Docker .............. v27.4.1 (daemon running)
  Podman .............. not installed
  Firecracker ......... not available (no KVM)
  Apple Containers .... macOS 26.0

Daemon:
  Status .............. running (socket: /tmp/agentkernel-daemon.sock)
  Warm VMs ............ 3/5
  Memory .............. 1.2 GB / 16 GB

Sandboxes:
  Running ............. 2
  Stopped ............. 5
  Stale (>7d) ......... 3 (run `agentkernel gc` to clean)

Disk:
  Images .............. 847 MB (~/.local/share/agentkernel/)
  Audit log ........... 2.1 MB (4,312 entries)

Config:
  agentkernel.toml .... valid
```

Extends existing `check_installation()` in `setup.rs`. Files: `main.rs`, `setup.rs`.

### Shell completions

```
$ agentkernel completions zsh > ~/.zfunc/_agentkernel
$ agentkernel completions bash > /etc/bash_completion.d/agentkernel
$ agentkernel completions fish > ~/.config/fish/completions/agentkernel.fish
```

Uses `clap_complete`. Files: `main.rs`, `Cargo.toml`.

### `stats` command

Local analytics from the existing audit log.

```
$ agentkernel stats

Executions:     1,247 total (last 30d: 892)
Sandboxes:      34 created, 29 removed, 5 active
Avg duration:   3.2s per exec
Top images:     python:3.12-alpine (412), node:22-alpine (287), rust:1.85 (98)
Top backends:   Docker (1,102), Apple (145)
First entry:    2026-01-20  Last: 2026-02-01

$ agentkernel stats --json   # machine-readable output
```

Requires adding `duration_ms: Option<u64>` to `AuditEvent::CommandExecuted`. Files: `main.rs`, new `stats.rs`, `audit.rs`.

### Config presets

```
$ agentkernel init --template claude-agent
$ agentkernel init --template python-ml
$ agentkernel init --template rust-ci
$ agentkernel init --template secure
$ agentkernel init --template node-fullstack
```

Each preset generates a tuned `agentkernel.toml`. For example, `claude-agent` sets `compatibility_mode = "claude"`, enables CWD mount, allows API domains; `secure` sets restrictive profile, no network, read-only root.

Files: `main.rs`, `config.rs`.

## P1: Core Enhancements

### Sandbox templates

User-created reusable sandbox configurations saved locally.

```
$ agentkernel template save my-python --from my-running-sandbox
$ agentkernel template list
  my-python       python:3.12-alpine  moderate  512MB
  ci-runner       rust:1.85-alpine    restrictive  1GB
$ agentkernel create --template my-python new-sandbox
```

Templates are TOML + optional Dockerfile stored in `~/.local/share/agentkernel/templates/`. Different from config presets — presets are built-in, templates are user-created from running sandboxes.

Files: new `template.rs`, `main.rs`.

### Agent-ready templates

Built-in templates for sandboxes with AI agents pre-installed and credential injection configured.

```
$ agentkernel init --template claude-sandbox
$ agentkernel init --template codex-sandbox
$ agentkernel init --template gemini-sandbox
$ agentkernel init --template opencode-sandbox
```

Each agent template includes:
- **Base image** with the agent CLI pre-installed (or install script in `[[files]]`)
- **Credential passthrough** — agent API keys injected from secrets vault or env:
  ```toml
  [agent]
  preferred = "claude"
  secrets = ["ANTHROPIC_API_KEY"]
  compatibility_mode = "claude"

  [security]
  profile = "moderate"

  [security.network.domains]
  allow = ["api.anthropic.com"]
  ```
- **Agent-specific defaults** — correct working directory mounts, PTY support, terminal size
- **Quick-start instructions** printed after init

Template details per agent:

| Template | Agent CLI | Required Secret | Allowed Domains |
|----------|-----------|-----------------|-----------------|
| `claude-sandbox` | Claude Code | `ANTHROPIC_API_KEY` | `api.anthropic.com`, `sentry.io` |
| `codex-sandbox` | OpenAI Codex | `OPENAI_API_KEY` | `api.openai.com` |
| `gemini-sandbox` | Gemini CLI | `GOOGLE_API_KEY` or `GEMINI_API_KEY` | `generativelanguage.googleapis.com` |
| `opencode-sandbox` | OpenCode | `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` | `api.openai.com`, `api.anthropic.com` |

Combined with the secrets vault (below), this gives a one-command flow:

```
$ agentkernel secret set ANTHROPIC_API_KEY
$ agentkernel init --template claude-sandbox
$ agentkernel run --keep "claude"
# Claude Code is running inside the sandbox with credentials injected
```

Files: `config.rs` (template definitions), `secrets.rs` (injection), `main.rs`.

### Auto-cleanup (TTL / GC)

Prevent sandbox sprawl with automatic expiry.

```
$ agentkernel run --ttl 1h "pytest"           # auto-remove after 1 hour
$ agentkernel create my-temp --ttl 24h        # expires in 24 hours
$ agentkernel gc                               # remove all expired sandboxes
$ agentkernel gc --dry-run                     # show what would be removed
```

Adds `ttl` and `last_active` to `SandboxState`. Daemon mode can run periodic GC in the background.

Files: `vmm.rs`, `main.rs`.

### `benchmark` command

Compare backends on your hardware.

```
$ agentkernel benchmark

Backend          Boot (cold)  Boot (warm)  Exec    Memory
Docker           220ms        45ms         12ms    48MB
Apple            940ms        -            18ms    64MB
Firecracker      125ms        8ms          5ms     25MB

$ agentkernel benchmark --backends docker,apple --iterations 10
```

Standard workload: create, start, exec(`echo hello`), stop, remove. Reports p50/p95 when `--iterations` > 1.

Files: new `benchmark.rs`, `main.rs`.

### Secrets vault

CRUD for API keys with pluggable backends. Secrets are auto-injected as env vars into sandboxes.

```
$ agentkernel secret set OPENAI_API_KEY
Enter value: ********
Stored (encrypted, backend: keyring)

$ agentkernel secret list
  OPENAI_API_KEY      keyring     set 2026-01-30
  ANTHROPIC_API_KEY   vault       set 2026-01-28
  GEMINI_API_KEY      1password   set 2026-01-25

$ agentkernel secret get OPENAI_API_KEY
$ agentkernel secret delete OPENAI_API_KEY
```

**Backends:**

| Backend | Config | Use Case |
|---------|--------|----------|
| `keyring` (default) | OS keyring (macOS Keychain, Linux secret-service) | Local dev, single machine |
| `env` | Read from environment variables | CI/CD, simple setups |
| `file` | `age`-encrypted file (`~/.agentkernel/secrets.age`) | Portable, no OS deps |
| `vault` | HashiCorp Vault (`VAULT_ADDR` + token/AppRole) | Teams, rotation, audit trail |
| `kubernetes` | K8s Secrets in configured namespace | K8s-deployed sandboxes |
| `nomad` | Nomad Variables in configured namespace | Nomad-deployed sandboxes |
| `1password` | 1Password CLI (`op`) or Connect API | Personal/team password manager |

Config reference:

```toml
[secrets]
backend = "vault"                          # default: "keyring"
keys = ["OPENAI_API_KEY", "ANTHROPIC_API_KEY"]

# Vault backend
[secrets.vault]
addr = "https://vault.example.com"         # or VAULT_ADDR env
mount = "secret"                           # KV v2 mount path
path = "agentkernel/api-keys"              # secret path
auth = "token"                             # token, approle, kubernetes

# Kubernetes backend
[secrets.kubernetes]
namespace = "agentkernel"                  # namespace for Secret objects
secret_name = "agent-api-keys"            # K8s Secret name

# Nomad backend
[secrets.nomad]
namespace = "default"
path = "agentkernel/api-keys"

# 1Password backend
[secrets.onepassword]
vault = "Development"                      # 1Password vault name
connect_host = "http://localhost:8080"     # optional: Connect server
```

The backend is selected per-project via config. `agentkernel secret set/get/list/delete` works the same regardless of backend — the CLI abstracts the storage layer.

When using orchestration backends (K8s, Nomad), secrets are mounted natively into pods/allocations rather than passed as env vars, avoiding exposure in process listings.

Files: new `secrets.rs` (trait + backends), `config.rs`, `vmm.rs`. Vault/1Password backends behind feature flags.

### `info` command

Detailed view of a single sandbox.

```
$ agentkernel info my-sandbox

Name:           my-sandbox
Status:         running (uptime: 2h 14m)
Backend:        docker
Image:          python:3.12-alpine
Resources:      2 vCPUs, 512MB RAM
Profile:        moderate
Network:        enabled (allow: api.openai.com, pypi.org)
Created:        2026-02-01 10:30:00
Last active:    2026-02-01 12:44:12

Recent activity (last 5):
  12:44:12  exec  python train.py         exit=0  3.2s
  12:40:01  exec  pip install torch       exit=0  12.1s
  12:38:55  file  wrote /app/config.yaml
  10:30:01  start
  10:30:00  create
```

Combines `VmManager::get_state()` + `AuditLog::read_by_sandbox()`. Files: `main.rs`.

## P2: Workflow Features

### Snapshot / restore

Save and resume sandbox state.

```
$ agentkernel snapshot my-sandbox --name before-refactor
$ agentkernel snapshot list
  before-refactor   my-sandbox   2026-02-01  docker  247MB
$ agentkernel restore before-refactor --as my-sandbox-v2
```

Implementation varies by backend:
- **Docker**: `docker commit` + `docker save`
- **Firecracker**: Firecracker snapshot API (VM memory + disk state)
- **Apple Containers**: filesystem snapshot

Snapshots stored in `~/.local/share/agentkernel/snapshots/`. Most valuable for long-running agent sessions where you want a checkpoint before a risky operation.

Files: `vmm.rs`, backend trait additions, `main.rs`.

### Agent sessions

Tie sandbox lifecycle to an agent conversation.

```
$ agentkernel session start --agent claude --name feature-x
  Created sandbox: session-feature-x
  MCP tools routed to session sandbox

$ agentkernel session list
  feature-x    claude    running    2h 14m    12 execs
  debug-auth   codex     stopped    yesterday

$ agentkernel session save feature-x
$ agentkernel session resume feature-x
```

Sessions bundle: sandbox name, audit trail, injected files, env vars, agent type. Resume recreates from saved state + snapshot if available.

Files: new `session.rs`, `mcp.rs`, `main.rs`.

### Per-branch sandboxes

Automatic sandbox naming from git context.

```
$ git checkout feature/auth
$ agentkernel run --branch "pytest"
  sandbox: myproject-feature-auth (auto-created)

$ git checkout main
$ agentkernel run --branch "pytest"
  sandbox: myproject-main (reuses existing)

$ agentkernel list --project
  myproject-feature-auth   running    docker
  myproject-main           stopped    docker
  myproject-fix-bug-42     stopped    docker
```

Files: new `git_utils.rs`, `main.rs`.

### Agent pipelines

Chain sandboxes with data flowing between steps.

```toml
# pipeline.toml
[[step]]
name = "generate"
image = "python:3.12-alpine"
command = "python generate_data.py"
output = "/app/output/"

[[step]]
name = "process"
image = "node:22-alpine"
command = "node process.js"
input = "/app/input/"
output = "/app/results/"

[[step]]
name = "analyze"
image = "python:3.12-alpine"
command = "python analyze.py"
input = "/app/input/"
```

```
$ agentkernel pipeline run pipeline.toml
  [1/3] generate .......... done (3.2s)
  [2/3] process ........... done (1.8s)
  [3/3] analyze ........... done (0.9s)
  Done (5.9s total)
```

Each step runs in its own sandbox. Output dir from step N is volume-mounted as input in step N+1.

Files: new `pipeline.rs`, `main.rs`.

## P3: Nice to Have

### Image cache management

```
$ agentkernel images list
  python:3.12-alpine    45MB    used by 3 sandboxes
  node:22-alpine        52MB    used by 1 sandbox
  agentkernel-myproj    128MB   custom build
  Total: 847MB

$ agentkernel images prune              # remove unused
$ agentkernel images pull rust:1.85     # pre-pull
```

### Artifact extraction

Extend `cp` with bulk operations.

```
$ agentkernel cp my-sandbox:/app/output/ ./local-output/     # recursive
$ agentkernel export my-sandbox --output my-env.tar          # full filesystem
```

### Sandbox export / import

Share sandbox configurations between machines.

```
$ agentkernel export-config my-sandbox > my-env.toml
$ agentkernel import-config my-env.toml --as new-sandbox
```

### Parallel execution

Fan-out independent jobs, fan-in results.

```
$ agentkernel parallel \
    --job "lint:node:22-alpine:npm run lint" \
    --job "test:python:3.12-alpine:pytest" \
    --job "build:rust:1.85:cargo build --release"

  lint done (2.1s)  test done (8.3s)  build done (45.2s)
  All jobs passed (45.2s wall time)
```

## Priority Matrix

| Feature | Value | Effort | Priority |
|---------|-------|--------|----------|
| `doctor` command | High | Low | **P0** |
| Shell completions | High | Very Low | **P0** |
| `stats` command | High | Low | **P0** |
| Config presets | Medium | Low | **P0** |
| Sandbox templates | High | Medium | **P1** |
| Agent-ready templates | High | Medium | **P1** |
| Auto-cleanup (TTL/GC) | High | Medium | **P1** |
| `benchmark` command | Medium | Low | **P1** |
| Secrets vault | High | Medium | **P1** |
| `info` command | Medium | Low | **P1** |
| Snapshot/restore | High | High | **P2** |
| Agent sessions | High | High | **P2** |
| Per-branch sandboxes | Medium | Low | **P2** |
| Agent pipelines | Medium | High | **P2** |
| Image cache management | Medium | Low | **P3** |
| Artifact extraction | Medium | Low | **P3** |
| Sandbox export/import | Low | Low | **P3** |
| Parallel execution | Medium | High | **P3** |

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                    agentkernel CLI                        │
│                                                          │
│  doctor  stats  benchmark  init  create  run  exec  ...  │
└───────────────────────┬──────────────────────────────────┘
                        │
          ┌─────────────┼─────────────┐
          ▼             ▼             ▼
   ┌─────────────┐ ┌────────┐ ┌───────────┐
   │  Templates  │ │ Secrets│ │  Sessions  │
   │  ~/.local/  │ │ vault  │ │  (agent    │
   │  share/     │ │        │ │  lifecycle)│
   └─────────────┘ └────────┘ └───────────┘
          │             │             │
          └─────────────┼─────────────┘
                        ▼
          ┌──────────────────────────┐
          │       VmManager          │
          │                          │
          │  TTL / GC    Snapshots   │
          │  Audit log   Pipelines   │
          └────────────┬─────────────┘
                       │
        ┌──────────────┼──────────────────┐
        ▼              ▼                  ▼
  ┌──────────┐  ┌────────────┐    ┌────────────┐
  │  Docker  │  │ Firecracker│    │   Apple     │
  │  Podman  │  │ Hyperlight │    │ Containers  │
  └──────────┘  └────────────┘    └────────────┘
                       │
                ┌──────┴──────┐
                │   Daemon    │
                │  (warm pool)│
                └─────────────┘
```

All features run locally. No remote host, no cloud account, no billing.
