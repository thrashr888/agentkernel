---
title: "AgentKernel: sandboxes for AI coding agents"
description: Run AI agent commands with local or hosted sandbox backends through a CLI, HTTP API, MCP server, and SDKs. Compare options and start your first sandbox.
---

# agentkernel

**Run AI coding agents in secure, isolated microVMs.**

AI coding agents execute arbitrary code on your machine. They install packages, modify files, run scripts, and shell out to system commands. That's what makes them useful -- and dangerous. A single hallucinated `rm -rf` or a compromised dependency runs with your full permissions, your credentials, your SSH keys.

AgentKernel gives you a choice of execution backends. Firecracker provides a dedicated guest kernel on Linux/KVM; Apple Containers provides VM-backed Linux containers on supported Macs. Docker and Podman use container isolation. Choose the backend and permissions that fit your workload.

## Find your starting point

- [Create your first sandbox](getting-started/quick-start.md) after [installation](getting-started/installation.md).
- [Choose a sandbox for AI coding agents](getting-started/choosing-a-sandbox.md) by execution location, isolation, and integration.
- Compare AgentKernel with [E2B](comparisons/e2b.md), [Daytona](comparisons/daytona.md), or [Docker Sandboxes](comparisons/docker.md).
- [Connect an assistant through MCP](api/mcp.md) or follow the [coding agent workflow](use-cases/coding-agents.md).

## Measure the workflow you run

Startup, pool acquisition, command execution, and full sandbox cleanup have
different costs. AgentKernel includes a benchmark command and backend-specific
harnesses so you can measure the path your agent uses. See the
[benchmark methodology and historical results](getting-started/benchmarks.md)
before comparing warm pools with cold starts.

## It's simple

If you've used Docker, you already know the CLI:

```bash
# Install
brew tap thrashr888/tap && brew install agentkernel
# Or: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
agentkernel setup

# Run any command in an isolated sandbox
agentkernel run python3 -c "print('Hello from sandbox!')"
agentkernel run node -e "console.log(1 + 1)"

# Create from a template
agentkernel sandbox create my-project --template python
agentkernel sandbox start my-project
agentkernel exec my-project -- pytest

# Or auto-name from your git branch
agentkernel sandbox create --branch -B docker
```

agentkernel auto-detects the runtime from your command or project files. Run `python3` and it pulls `python:3.12-alpine`. Run `cargo build` and it pulls `rust:1.85-alpine`. Image selection covers JavaScript, Python, Rust, Go, Ruby, Java, C#, C/C++, PHP, Elixir, Terraform, and Shell. Project files and dependencies still need to be supplied explicitly; the moderate profile does not mount your working directory. See the [coding agent workflow](use-cases/coding-agents.md).

## Connect your coding agent

Claude Code, Codex, Gemini CLI, GitHub Copilot, Amp, OpenCode, Pi -- agentkernel runs them all. Each agent gets its own isolated sandbox with configurable security profiles.

```bash
# Check which agents are available
agentkernel agents

# Run Claude Code in a sandbox
agentkernel sandbox create my-project --config examples/agents/claude-code/agentkernel.toml
agentkernel sandbox start my-project
agentkernel attach my-project -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY
```

For Claude Code, install the project integration to make sandbox tools and the `/sandbox` command available. Other host tools keep their own permissions:

```bash
# In your project terminal
agentkernel plugin install claude

# Then in Claude Code
/sandbox python3 -c "print(1 + 1)"
```

## Security is configurable

Not every task needs maximum lockdown. agentkernel provides three security profiles that control network access, filesystem mounts, and environment passthrough:

| Profile | Network | Mount CWD | Mount Home | Pass Env | Read-only |
|---------|---------|-----------|------------|----------|-----------|
| **permissive** | Yes | Yes | Yes | Yes | No |
| **moderate** (default) | Yes | No | No | No | No |
| **restrictive** | No | No | No | No | Yes |

```bash
# Run with no network access and read-only filesystem
agentkernel run --profile restrictive python3 -c "print(1 + 1)"

# Or toggle individual settings
agentkernel run --no-network curl example.com  # Will fail
```

## Keep API keys on the host with proxy injection

AI agents need API keys to call LLMs, but putting secrets inside sandboxes defeats the purpose of isolation. A compromised agent could exfiltrate your `ANTHROPIC_API_KEY` to any host.

AgentKernel supports **network-layer secret injection** for compatible workloads. A host-side HTTPS proxy intercepts outbound requests and injects credentials at the network layer, scoped to specific domains. The real secret never crosses the VM boundary.

```bash
# Inject API key into requests to api.openai.com only
agentkernel sandbox create my-agent --secret OPENAI_API_KEY:api.openai.com

# Inside the sandbox:
# curl https://api.openai.com/v1/models → Authorization header injected
# curl https://evil.com → blocked (403)
# echo $OPENAI_API_KEY → "ak-proxy-managed" (placeholder)
```

With proxy injection, the sandbox sees placeholder environment values and the real key stays on the host. The proxy restricts the destinations it forwards to. File injection and explicit environment passthrough have a different contract: the workload can read those real credentials. See [secret delivery methods](features/secrets.md), and note that full-state Firecracker checkpoints currently reject sandboxes using host-side proxies.

## It runs everywhere

agentkernel covers local, cluster, and hosted backends behind the same CLI and HTTP API.

### Local backends

| Platform | Backend | Isolation |
|----------|---------|-----------|
| Linux (x86_64, aarch64) | Firecracker microVMs | Full VM isolation via KVM |
| Linux (x86_64) | Hyperlight Wasm | Hypervisor + Wasm sandbox (experimental; [arm64 probe](operations/dependency-compatibility.md#hyperlight-arm64-exception)) |
| macOS 26+ (Apple Silicon) | Apple Containers | Full VM isolation |
| macOS (Apple Silicon, Intel) | Docker / Podman | Container isolation |

### Cluster backends

| Target | Backend | Isolation |
|--------|---------|-----------|
| Kubernetes cluster | K8s Pods | Pod isolation + NetworkPolicy |
| Nomad cluster | Nomad Jobs | Job allocation isolation |

### Hosted remote backends

| Provider | Backend | Notes |
|----------|---------|-------|
| Daytona | `daytona` | Hosted sandbox with managed `/workspace` sync |
| Runloop | `runloop` | Hosted devbox with tunnels, attach, and snapshot/restore |
| E2B | `e2b` | Hosted sandbox with file APIs, PTY attach, and snapshots |
| Modal | `modal` | Hosted sandbox with tunnels, attach, and workspace snapshots |
| Agent Computer | `agentcomputer` | Contract wired; live bundled adapter still pending |

On Linux with KVM and an installed Firecracker runtime, automatic selection prefers Firecracker. On macOS 26+, Apple Containers provide native VM isolation. On older macOS or systems without KVM, Docker and Podman provide container-level isolation as a fallback. For team and cloud environments, deploy on [Kubernetes](operations/kubernetes.md) or [Nomad](operations/nomad.md) with warm pools, CRDs, and Helm/Nomad Pack support.

For team and multi-tenant deployments, Kubernetes, Nomad, Daytona, Runloop, E2B, and Modal keep the same sandbox lifecycle and command surface while moving execution off your laptop. See the [Backends Guide](config/backends.md) for the full matrix and the [Remote Backends Guide](operations/remote.md) for hosted setup, templates, and examples.

```bash
# Run on Kubernetes
agentkernel run --backend kubernetes -- python3 -c "print('hello from k8s')"

# Run on Nomad
agentkernel run --backend nomad -- echo "hello from nomad"

# Run on Daytona
agentkernel sandbox create remote-daytona --backend daytona

# Run on Runloop
agentkernel sandbox create remote-runloop --backend runloop

# Run on E2B
agentkernel sandbox create remote-e2b --backend e2b

# Run on Modal
agentkernel sandbox create remote-modal --backend modal
```

Cluster backends offer warm-pool workflows; capacity and latency depend on the cluster and workload. Hosted backends use the same sandbox lifecycle, with provider-side execution and managed `/workspace` sync.

## It has a complete workflow

Templates, snapshots, sessions, pipelines, and parallel execution — everything you need for real development workflows.

```bash
# Templates: pre-configured sandbox environments
agentkernel sandbox create ci --template rust-ci

# Snapshots: save and restore sandbox state
agentkernel snapshot take my-sandbox --name before-upgrade
agentkernel snapshot restore before-upgrade --as rollback

# Firecracker full-state lifecycle: preserve memory and running processes
agentkernel sandbox pause my-sandbox
agentkernel sandbox fork my-sandbox --as candidate-b
agentkernel sandbox resume my-sandbox

# Sessions: tie sandbox lifecycle to agent conversations
agentkernel session start --name feature-x --agent claude -B docker
agentkernel session save feature-x
agentkernel session resume feature-x

# Pipelines: chain sandboxes with data flow
agentkernel pipeline pipeline.toml

# Parallel: fan-out jobs across sandboxes
agentkernel parallel \
  --job "lint:node:22-alpine:npx eslint ." \
  --job "test:node:22-alpine:npm test"
```

Per-branch sandboxes, image cache management, secrets vault, sandbox export/import, TTL-based auto-expiry, and garbage collection round out the developer experience.

Filesystem snapshots and Firecracker full-state checkpoints have different
guarantees. Full-state pause/resume/fork remains a preview and is initially restricted to Firecracker
1.16.1 on compatible x86_64 Linux/KVM hosts and never silently falls back on
other backends. The CLI lifecycle commands delegate to a running
`agentkernel serve` process that owns the VMM. Read the [full-state
compatibility and operations guide](operations/firecracker-full-state.md) before
relying on process continuity or branching a live agent.

## It's programmable

Run agentkernel as an HTTP server for programmatic sandbox management:

```bash
# As a background service (recommended)
brew services start agentkernel

# Or run manually
agentkernel serve --host 127.0.0.1 --port 18888
```

```typescript
import { AgentKernel } from "agentkernel";

const client = new AgentKernel();

// Run a command in a temporary sandbox
const result = await client.run(["python3", "-c", "print(1+1)"]);
console.log(result.output); // "2\n"

// Sandbox session with automatic cleanup
await using sandbox = await client.sandbox("my-session");
await sandbox.exec(["npm", "install"]);
const tests = await sandbox.exec(["npm", "test"]);
```

Official SDKs for [Node.js](sdks/nodejs.md), [Python](sdks/python.md), [Go](sdks/golang.md), [Rust](sdks/rust.md), and [Swift](sdks/swift.md). Full REST API for creating, managing, and executing commands in sandboxes. Build agent orchestration systems, CI/CD pipelines, or interactive coding environments on top of agentkernel.

| SDK | Package | Install |
|-----|---------|---------|
| [Node.js](sdks/nodejs.md) | [`agentkernel`](https://www.npmjs.com/package/agentkernel) | `npm install agentkernel` |
| [Python](sdks/python.md) | [`agentkernel-sdk`](https://pypi.org/project/agentkernel-sdk/) | `pip install agentkernel-sdk` |
| [Go](sdks/golang.md) | [`agentkernel`](https://pkg.go.dev/github.com/thrashr888/agentkernel/sdk/golang) | `go get github.com/thrashr888/agentkernel/sdk/golang` |
| [Rust](sdks/rust.md) | [`agentkernel-sdk`](https://crates.io/crates/agentkernel-sdk) | `cargo add agentkernel-sdk` |
| [Swift](sdks/swift.md) | `AgentKernel` | Swift Package Manager |

## Enterprise policy management

For organizations that need centralized control over what agents can do, agentkernel supports Cedar-based policy management with cryptographic signing, RBAC, and compliance audit logging.

```toml
# agentkernel.toml
[enterprise]
enabled = true
policy_server = "https://policy.your-company.com"
org_id = "acme-corp"
offline_mode = "cached_with_expiry"

[enterprise.trust_anchors]
keys = ["prod-signing-key-2026"]
```

Policies are written in [Cedar](https://www.cedarpolicy.com/), Amazon's open-source authorization language. Default deny -- if no policy permits an action, it's blocked.

```
// Only developers can create sandboxes
permit(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Create",
    resource is AgentKernel::Sandbox
) when {
    principal.roles.contains("developer")
};

// Network access requires MFA
forbid(
    principal is AgentKernel::User,
    action == AgentKernel::Action::"Network",
    resource is AgentKernel::Sandbox
) when {
    !principal.mfa_verified
};
```

Every policy decision is logged in OCSF-compatible JSONL for compliance auditing (SOC 2, HIPAA, FedRAMP). Policies are signed with Ed25519 to prevent tampering, with version monotonicity checks to block downgrades.

### Desktop policy activation

The desktop app starts its app-owned Local sidecar with one canonical absolute
configuration path under the AgentKernel desktop configuration directory. The
Policy page reports whether the enterprise feature is compiled, configured,
active, enforcing, and healthy, along with the policy source, version, and any
initialization error.

Only an app-managed loopback Local server can be activated from the desktop.
Remote and separately managed servers are strictly read-only; ask the server
administrator to change their Cedar policy. Local activation validates the
TOML and Cedar text first, then atomically replaces `agentkernel.toml` and
`policy.cedar`, preserving `.bak` copies. If the restarted sidecar does not
report active, enforcing, and healthy, both files are restored and the prior
configuration is restarted.

When no meaningful policy material is available in a non-fail-closed offline
mode, the compatibility `default_permit_all` fallback is labeled explicitly.
It is not considered policy enforcement. Use a local Cedar file or a managed
policy server for meaningful authorization.

Build with `cargo build --features enterprise`. See [example policies](https://github.com/thrashr888/agentkernel/tree/main/examples/enterprise/policies) for RBAC, MFA enforcement, runtime restrictions, and org isolation patterns.

## Docker vs. AgentKernel

AgentKernel's Docker backend runs ordinary containers. Select Firecracker or
Apple Containers when you need their VM boundaries and meet their host
requirements. Docker's separate Sandboxes product also uses microVMs; it is not
the same thing as an ordinary Docker container.

The [Docker Sandboxes comparison](comparisons/docker.md) explains these choices
and the CLI, API, and MCP workflows. No isolation backend protects files and
credentials you deliberately share with it.

## Get started

```bash
brew tap thrashr888/tap && brew install agentkernel
# Or: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
agentkernel setup
agentkernel run python3 -c "print('Hello from sandbox!')"
```

- [Installation](getting-started/installation.md) - Detailed setup instructions
- [Getting Started](getting-started/quick-start.md) - Your first sandbox
- [Commands](commands/index.md) - Full CLI reference
- [Configuration](config/index.md) - Config file format
- [Templates](commands/templates.md) - Pre-configured sandbox environments
- [Snapshots](commands/snapshots.md) - Save and restore sandbox state
- [Sessions](commands/sessions.md) - Agent session lifecycle management
- [Pipelines](commands/pipelines.md) - Multi-step sandbox pipelines
- [Secrets](commands/secrets.md) - API key and credential management
- [Agents](agents/index.md) - Running Claude Code, Codex, Gemini CLI
- [HTTP API](api/index.md) - Programmatic access
- [SDKs](sdks/index.md) - Client libraries for [Node.js](sdks/nodejs.md), [Python](sdks/python.md), [Go](sdks/golang.md), [Rust](sdks/rust.md), [Swift](sdks/swift.md)
- [Benchmarks](getting-started/benchmarks.md) - Measurement methods and historical results
- [Comparisons](getting-started/comparisons.md) - How agentkernel compares to E2B, Daytona, Docker, and others
