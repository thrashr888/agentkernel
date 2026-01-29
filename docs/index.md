
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
curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
agentkernel setup

# Run any command in an isolated sandbox
agentkernel run python3 -c "print('Hello from sandbox!')"
agentkernel run npm test
agentkernel run cargo build

# Create a persistent sandbox for longer work
agentkernel create my-project --dir .
agentkernel start my-project
agentkernel exec my-project pytest
```

agentkernel auto-detects the runtime from your command or project files. Run `python3` and it pulls `python:3.12-alpine`. Run `cargo build` and it pulls `rust:1.85-alpine`. No configuration needed for 12+ languages -- JavaScript, Python, Rust, Go, Ruby, Java, C#, C/C++, PHP, Elixir, Terraform, and Shell.

## It works with every agent

Claude Code, Codex, Gemini CLI, OpenCode -- agentkernel runs them all. Each agent gets its own isolated sandbox with configurable security profiles.

```bash
# Check which agents are available
agentkernel agents

# Run Claude Code in a sandbox
agentkernel create my-project --config examples/agents/claude-code/agentkernel.toml
agentkernel start my-project
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

## It runs everywhere

agentkernel picks the best available backend automatically:

| Platform | Backend | Isolation |
|----------|---------|-----------|
| Linux (x86_64, aarch64) | Firecracker microVMs | Full VM isolation via KVM |
| Linux (x86_64, aarch64) | Hyperlight Wasm | Hypervisor + Wasm sandbox (experimental) |
| macOS 26+ (Apple Silicon) | Apple Containers | Full VM isolation |
| macOS (Apple Silicon, Intel) | Docker / Podman | Container isolation |

On Linux with KVM, you get Firecracker -- the same microVM technology that powers AWS Lambda and Fargate. On macOS 26+, Apple Containers provide native VM isolation. On older macOS or systems without KVM, Docker and Podman provide container-level isolation as a fallback.

## It's programmable

Run agentkernel as an HTTP server for programmatic sandbox management:

```bash
agentkernel serve --host 127.0.0.1 --port 8080
```

```bash
# Run a command via the API
curl -X POST http://localhost:8080/run \
  -H "Content-Type: application/json" \
  -d '{"command": ["python3", "-c", "print(1+1)"], "profile": "restrictive"}'

# Response: {"success": true, "data": {"output": "2\n"}}
```

Full REST API for creating, managing, and executing commands in sandboxes. Build agent orchestration systems, CI/CD pipelines, or interactive coding environments on top of agentkernel.

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
curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
agentkernel setup
agentkernel run python3 -c "print('Hello from sandbox!')"
```

- [Installation](installation.html) - Detailed setup instructions
- [Getting Started](getting-started.html) - Your first sandbox
- [Commands](commands.html) - Full CLI reference
- [Configuration](configuration.html) - Config file format
- [Agents](agents.html) - Running Claude Code, Codex, Gemini CLI
- [HTTP API](api.html) - Programmatic access
- [SDKs](sdks.html) - Client libraries for [Node.js](sdk-nodejs.html), [Python](sdk-python.html), [Go](sdk-golang.html), [Rust](sdk-rust.html), [Swift](sdk-swift.html)
- [Benchmarks](benchmarks.html) - Performance numbers for every backend
- [Comparisons](comparisons.html) - How agentkernel compares to E2B, Daytona, Docker, and others
