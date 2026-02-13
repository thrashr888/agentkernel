
# agentkernel

**Run AI coding agents in secure, isolated microVMs.**

AI coding agents execute arbitrary code on your machine. They install packages, modify files, run scripts, and shell out to system commands. That's what makes them useful -- and dangerous. A single hallucinated `rm -rf` or a compromised dependency runs with your full permissions, your credentials, your SSH keys.

Docker helps, but it shares the host kernel. Container escapes are not theoretical -- they're documented CVEs. When the threat model is "an AI is running arbitrary code," you need stronger isolation than a namespace boundary.

agentkernel gives each sandbox its own virtual machine with a dedicated Linux kernel. Hardware-enforced memory boundaries via KVM. No shared kernel, no container escapes, no attack surface beyond the hypervisor. The same isolation model behind AWS Lambda (Firecracker), now available as a single binary for your dev machine.

## It's fast

The usual knock on VMs is startup time. agentkernel sidesteps this entirely:

| Mode | Latency |
|------|---------|
| Hyperlight pool (pre-warmed) | **<1&micro;s** |
| Hyperlight (cold start) | ~41ms |
| Firecracker daemon (warm pool) | ~195ms |
| Docker (macOS) | ~220ms |
| Podman (macOS) | ~300ms |

Pre-warmed VM pools make execution feel instant. Cold starts are still faster than most container runtimes. The daemon maintains 3-5 pre-booted Firecracker VMs so commands execute in ~195ms vs ~800ms for cold starts -- a 4x speedup.

## It's simple

If you've used Docker, you already know the CLI:

```bash
# Install
brew tap thrashr888/agentkernel && brew install agentkernel
# Or: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
agentkernel setup

# Run any command in an isolated sandbox
agentkernel run python3 -c "print('Hello from sandbox!')"
agentkernel run npm test
agentkernel run cargo build

# Create from a template
agentkernel sandbox create my-project --template python
agentkernel sandbox start my-project
agentkernel exec my-project -- pytest

# Or auto-name from your git branch
agentkernel sandbox create --branch -B docker
```

agentkernel auto-detects the runtime from your command or project files. Run `python3` and it pulls `python:3.12-alpine`. Run `cargo build` and it pulls `rust:1.85-alpine`. No configuration needed for 12+ languages -- JavaScript, Python, Rust, Go, Ruby, Java, C#, C/C++, PHP, Elixir, Terraform, and Shell.

## It works with every agent

Claude Code, Codex, Gemini CLI, GitHub Copilot, Amp, OpenCode, Pi -- agentkernel runs them all. Each agent gets its own isolated sandbox with configurable security profiles.

```bash
# Check which agents are available
agentkernel agents

# Run Claude Code in a sandbox
agentkernel sandbox create my-project --config examples/agents/claude-code/agentkernel.toml
agentkernel sandbox start my-project
agentkernel attach my-project -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY
```

For Claude Code specifically, agentkernel ships as a plugin. Install it and Claude automatically sandboxes risky operations:

```bash
# In Claude Code
/plugin install sandbox@thrashr888/agentkernel
/sandbox npm test
/sandbox cargo build
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
agentkernel run --profile restrictive python3 script.py

# Or toggle individual settings
agentkernel run --no-network curl example.com  # Will fail
```

## Secrets never enter the VM

AI agents need API keys to call LLMs, but putting secrets inside sandboxes defeats the purpose of isolation. A compromised agent could exfiltrate your `ANTHROPIC_API_KEY` to any host.

agentkernel solves this with **network-layer secret injection** -- the Gondolin pattern. A host-side HTTPS proxy intercepts outbound requests and injects credentials at the network layer, scoped to specific domains. The real secret never crosses the VM boundary.

```bash
# Inject API key into requests to api.openai.com only
agentkernel sandbox create my-agent --secret OPENAI_API_KEY:api.openai.com

# Inside the sandbox:
# curl https://api.openai.com/v1/models → Authorization header injected
# curl https://evil.com → blocked (403)
# echo $OPENAI_API_KEY → "ak-proxy-managed" (placeholder)
```

The sandbox sees placeholder env vars so tools pass existence checks, but the real key stays on the host. Unauthorized domains are blocked. No other sandbox runtime offers this -- most inject secrets as env vars or mounted files, which a compromised agent can read and exfiltrate.

## It runs everywhere

agentkernel picks the best available backend automatically:

| Platform | Backend | Isolation |
|----------|---------|-----------|
| Linux (x86_64, aarch64) | Firecracker microVMs | Full VM isolation via KVM |
| Linux (x86_64, aarch64) | Hyperlight Wasm | Hypervisor + Wasm sandbox (experimental) |
| macOS 26+ (Apple Silicon) | Apple Containers | Full VM isolation |
| macOS (Apple Silicon, Intel) | Docker / Podman | Container isolation |
| Kubernetes cluster | K8s Pods | Pod isolation + NetworkPolicy |
| Nomad cluster | Nomad Jobs | Job allocation isolation |

On Linux with KVM, you get Firecracker -- the same microVM technology that powers AWS Lambda and Fargate. On macOS 26+, Apple Containers provide native VM isolation. On older macOS or systems without KVM, Docker and Podman provide container-level isolation as a fallback. For team and cloud environments, deploy on [Kubernetes](operations/kubernetes.md) or [Nomad](operations/nomad.md) with warm pools, CRDs, and Helm/Nomad Pack support.

For team and multi-tenant deployments, Kubernetes and Nomad backends run sandboxes on remote clusters. Same CLI, same API -- sandboxes just run on your cluster instead of your laptop.

```bash
# Run on Kubernetes
agentkernel run --backend kubernetes -- python3 -c "print('hello from k8s')"

# Run on Nomad
agentkernel run --backend nomad -- echo "hello from nomad"
```

Both backends support warm pools for fast acquisition (~570ms one-shot latency) and scale to dozens of concurrent sandboxes per node.

## It has a complete workflow

Templates, snapshots, sessions, pipelines, and parallel execution — everything you need for real development workflows.

```bash
# Templates: pre-configured sandbox environments
agentkernel sandbox create ci --template rust-ci

# Snapshots: save and restore sandbox state
agentkernel snapshot take my-sandbox --name before-upgrade
agentkernel snapshot restore before-upgrade --as rollback

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

## It's programmable

Run agentkernel as an HTTP server for programmatic sandbox management:

```bash
# As a background service (recommended)
brew services start thrashr888/agentkernel/agentkernel

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

Build with `cargo build --features enterprise`. See [example policies](https://github.com/thrashr888/agentkernel/tree/main/examples/enterprise/policies) for RBAC, MFA enforcement, runtime restrictions, and org isolation patterns.

## Docker vs. agentkernel

The comparison people ask about most:

| | Docker | agentkernel |
|--|--------|-------------|
| **Kernel** | Shared with host | Dedicated per sandbox |
| **Escape risk** | Container escapes documented | Hardware-enforced isolation |
| **Boot time** | 1-5 seconds | <1&micro;s (warm pool) to ~220ms |
| **Memory overhead** | 50-100MB | <10MB |
| **Setup** | Docker Desktop or daemon | Single binary, no daemon required |

Docker is a great tool for packaging and deploying applications. agentkernel is purpose-built for running untrusted code. Different tools for different threat models.

## Get started

```bash
brew tap thrashr888/agentkernel && brew install agentkernel
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
- [Benchmarks](getting-started/benchmarks.md) - Performance numbers for every backend
- [Comparisons](getting-started/comparisons.md) - How agentkernel compares to E2B, Daytona, Docker, and others
