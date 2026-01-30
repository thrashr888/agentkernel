# agentkernel

Run AI coding agents in secure, isolated microVMs. Sub-125ms boot times, real hardware isolation.

## Installation

```bash
# Homebrew (macOS / Linux)
brew tap thrashr888/agentkernel && brew install agentkernel

# Or with the install script
curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh

# Or with Cargo
cargo install agentkernel

# Then run setup to download/build required components
agentkernel setup
```

## Quick Start

```bash
# Run any command in an isolated sandbox (auto-detects runtime)
agentkernel run python3 -c "print('Hello from sandbox!')"
agentkernel run node -e "console.log('Hello from sandbox!')"
agentkernel run ruby -e "puts 'Hello from sandbox!'"

# Run commands in your project
agentkernel run npm test
agentkernel run cargo build
agentkernel run pytest

# Run with a specific image
agentkernel run --image postgres:16-alpine psql --version
```

## The `run` Command

The fastest way to execute code in isolation. Creates a temporary sandbox, runs your command, and cleans up automatically.

```bash
# Auto-detects the right runtime from your command
agentkernel run python3 script.py      # Uses python:3.12-alpine
agentkernel run npm install            # Uses node:22-alpine
agentkernel run cargo test             # Uses rust:1.85-alpine
agentkernel run go build               # Uses golang:1.23-alpine

# Override with explicit image
agentkernel run --image ubuntu:24.04 apt-get --version

# Keep the sandbox after execution for debugging
agentkernel run --keep npm test

# Use a config file
agentkernel run --config ./agentkernel.toml npm test
```

## Auto-Detection

agentkernel automatically selects the right Docker image based on:

1. **Command** (for `run`) - Detects from the command you're running
2. **Project files** - Detects from files in your directory
3. **Procfile** - Parses Heroku-style Procfiles
4. **Config file** - Uses `agentkernel.toml` if present

### Supported Languages

| Language | Project Files | Commands | Docker Image |
|----------|--------------|----------|--------------|
| JavaScript/TypeScript | `package.json`, `yarn.lock`, `pnpm-lock.yaml` | `node`, `npm`, `npx`, `yarn`, `pnpm`, `bun` | `node:22-alpine` |
| Python | `pyproject.toml`, `requirements.txt`, `Pipfile` | `python`, `python3`, `pip`, `poetry`, `uv` | `python:3.12-alpine` |
| Rust | `Cargo.toml` | `cargo`, `rustc` | `rust:1.85-alpine` |
| Go | `go.mod` | `go`, `gofmt` | `golang:1.23-alpine` |
| Ruby | `Gemfile` | `ruby`, `bundle`, `rails` | `ruby:3.3-alpine` |
| Java | `pom.xml`, `build.gradle` | `java`, `mvn`, `gradle` | `eclipse-temurin:21-alpine` |
| Kotlin | `*.kt` | - | `eclipse-temurin:21-alpine` |
| C# / .NET | `*.csproj`, `*.sln` | `dotnet` | `mcr.microsoft.com/dotnet/sdk:8.0` |
| C/C++ | `Makefile`, `CMakeLists.txt` | `gcc`, `g++`, `make`, `cmake` | `gcc:14-bookworm` |
| PHP | `composer.json` | `php`, `composer` | `php:8.3-alpine` |
| Elixir | `mix.exs` | `elixir`, `mix` | `elixir:1.16-alpine` |
| Lua | `*.lua` | `lua`, `luajit` | `nickblah/lua:5.4-alpine` |
| HCL/Terraform | `*.tf`, `*.tfvars` | `terraform` | `hashicorp/terraform:1.10` |
| Shell | `*.sh` | `bash`, `sh`, `zsh` | `alpine:3.20` |

### Procfile Support

If your project has a `Procfile`, agentkernel parses it to detect the runtime:

```procfile
web: bundle exec rails server -p $PORT
worker: python manage.py runworker
```

## Persistent Sandboxes

For longer-running work, create named sandboxes:

```bash
# Create a sandbox
agentkernel create my-project --dir .

# Start it
agentkernel start my-project

# Run commands
agentkernel exec my-project npm test
agentkernel exec my-project python -m pytest

# Attach an interactive shell
agentkernel attach my-project

# Stop and remove
agentkernel stop my-project
agentkernel remove my-project

# List all sandboxes
agentkernel list
```

## Security Profiles

Control sandbox permissions with security profiles:

```bash
# Default: moderate security (network enabled, no mounts)
agentkernel run npm test

# Restrictive: no network, read-only filesystem, all capabilities dropped
agentkernel run --profile restrictive python3 script.py

# Permissive: network, mounts, environment passthrough
agentkernel run --profile permissive cargo build

# Disable network access specifically
agentkernel run --no-network curl example.com  # Will fail
```

| Profile | Network | Mount CWD | Mount Home | Pass Env | Read-only |
|---------|---------|-----------|------------|----------|-----------|
| permissive | Yes | Yes | Yes | Yes | No |
| moderate | Yes | No | No | No | No |
| restrictive | No | No | No | No | Yes |

## Configuration

Create `agentkernel.toml` in your project root:

```toml
[sandbox]
name = "my-project"
base_image = "python:3.12-alpine"    # Explicit Docker image

[agent]
preferred = "claude"    # claude, gemini, codex, opencode

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "restrictive"    # permissive, moderate, restrictive
network = false            # Override: disable network
```

Most projects don't need a config file - agentkernel auto-detects everything.

## HTTP API

Run agentkernel as an HTTP server for programmatic access:

```bash
# As a background service (recommended — survives reboots)
brew services start agentkernel

# Or run manually
agentkernel serve --host 127.0.0.1 --port 8080
```

### Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check |
| POST | `/run` | Run command in temporary sandbox |
| GET | `/sandboxes` | List all sandboxes |
| POST | `/sandboxes` | Create a sandbox |
| GET | `/sandboxes/{name}` | Get sandbox info |
| DELETE | `/sandboxes/{name}` | Remove sandbox |
| POST | `/sandboxes/{name}/exec` | Execute command in sandbox |

### Example

```bash
# Run a command
curl -X POST http://localhost:8080/run \
  -H "Content-Type: application/json" \
  -d '{"command": ["python3", "-c", "print(1+1)"], "profile": "restrictive"}'

# Response: {"success": true, "data": {"output": "2\n"}}
```

## Multi-Agent Support

Check which AI coding agents are available:

```bash
agentkernel agents
```

Output:
```
AGENT           STATUS          API KEY
---------------------------------------------
Claude Code     installed       set
Gemini CLI      not installed   missing
Codex           installed       set
OpenCode        installed       set
```

## SDKs

Official client libraries for the agentkernel HTTP API:

| SDK | Package | Install |
|-----|---------|---------|
| **Node.js** | [`agentkernel`](https://www.npmjs.com/package/agentkernel) | `npm install agentkernel` |
| **Python** | [`agentkernel`](https://pypi.org/project/agentkernel/) | `pip install agentkernel` |
| **Go** | [`agentkernel`](https://pkg.go.dev/github.com/thrashr888/agentkernel/sdk/golang) | `go get github.com/thrashr888/agentkernel/sdk/golang` |
| **Rust** | [`agentkernel-sdk`](https://crates.io/crates/agentkernel-sdk) | `cargo add agentkernel-sdk` |
| **Swift** | `AgentKernel` | Swift Package Manager |

```typescript
// Node.js
import { AgentKernel } from "agentkernel";
const client = new AgentKernel();
const result = await client.run(["echo", "hello"]);
```

```python
# Python
from agentkernel import AgentKernel
with AgentKernel() as client:
    result = client.run(["echo", "hello"])
```

```go
// Go
client := agentkernel.New(nil)
output, _ := client.Run(context.Background(), []string{"echo", "hello"}, nil)
```

```rust
// Rust
let client = agentkernel_sdk::AgentKernel::builder().build()?;
let output = client.run(&["echo", "hello"], None).await?;
```

```swift
// Swift
let client = AgentKernel()
let output = try await client.run(["echo", "hello"])
```

All SDKs support sandbox sessions with automatic cleanup, streaming output (SSE), and configuration via environment variables or explicit options. See [`sdk/`](sdk/) for full documentation.

## Why agentkernel?

AI coding agents execute arbitrary code. Running them directly on your machine is risky:
- They can read/modify any file
- They can access your credentials and SSH keys
- Container escapes are a real threat

agentkernel uses **Firecracker microVMs** (the same tech behind AWS Lambda) to provide true hardware isolation:

| Feature | Docker | agentkernel |
|---------|--------|-------------|
| Isolation | Shared kernel | Separate kernel per VM |
| Boot time | 1-5 seconds | <125ms |
| Memory overhead | 50-100MB | <10MB |
| Escape risk | Container escapes possible | Hardware-enforced isolation |

## Platform Support

| Platform | Backend | Status |
|----------|---------|--------|
| Linux (x86_64, aarch64) | Firecracker microVMs | Full support |
| Linux (x86_64, aarch64) | Hyperlight Wasm | Experimental |
| macOS 26+ (Apple Silicon) | Apple Containers | Full support (VM isolation) |
| macOS (Apple Silicon, Intel) | Docker | Full support (~220ms) |
| macOS (Apple Silicon, Intel) | Podman | Full support (~300ms) |

On macOS, agentkernel automatically selects the best available backend:
1. **Apple Containers** (macOS 26+) - True VM isolation, ~940ms
2. **Docker** - Fastest container option, ~220ms
3. **Podman** - Rootless/daemonless, ~300ms

Firecracker and Hyperlight require KVM (Linux only).

## Claude Code Integration

agentkernel includes a Claude Code skill plugin for seamless AI agent integration.

### Install the Plugin

```bash
# Add the marketplace and install (in Claude Code)
/plugin marketplace add thrashr888/agentkernel
/plugin install sandbox

# Or install directly
/plugin install sandbox@thrashr888/agentkernel
```

### Usage in Claude Code

Once installed, Claude will automatically use agentkernel for isolated execution:

- **Skill**: Claude detects when sandboxing is beneficial and uses the `sandbox` skill
- **Command**: Use `/sandbox <command>` to explicitly run in a sandbox

```
/sandbox python3 -c "print('Hello from sandbox!')"
/sandbox npm test
/sandbox cargo build
```

## Performance

| Mode | Platform | Latency | Use Case |
|------|----------|---------|----------|
| **Hyperlight Pool** | Linux | **<1µs** | Sub-microsecond with pre-warmed runtimes (experimental) |
| Hyperlight (cold) | Linux | ~41ms | Cold start Wasm runtime |
| Daemon (warm pool) | Linux | 195ms | API/interactive - fast with full VM isolation |
| Docker | macOS | ~220ms | macOS development (fastest) |
| Podman | macOS | ~300ms | macOS development (rootless) |
| Podman | Linux | ~310ms | Linux without KVM (fastest, daemonless) |
| Docker | Linux | ~350ms | Linux without KVM |
| Firecracker (cold) | Linux | ~800ms | One-off commands |

See [BENCHMARK.md](BENCHMARK.md) for detailed benchmarks and methodology.

## Daemon Mode (Linux)

For the fastest execution on Linux, use daemon mode to maintain a pool of pre-warmed VMs:

```bash
# Start the daemon (pre-warms 3 VMs)
agentkernel daemon start

# Run commands (uses warm VMs - ~195ms latency)
agentkernel run echo "Hello from warm VM!"

# Check pool status
agentkernel daemon status
# Output: Pool: Warm VMs: 3, In use: 0, Min/Max: 3/5

# Stop the daemon
agentkernel daemon stop
```

The daemon maintains 3-5 pre-booted Firecracker VMs. Commands execute in ~195ms vs ~800ms for cold starts - a **4x speedup**.

## Hyperlight Backend (Linux, Experimental)

Hyperlight uses Microsoft's hypervisor-isolated micro VMs to run WebAssembly with dual-layer security (Wasm sandbox + hypervisor boundary). This provides the fastest isolation with ~68ms latency.

**Requirements:**
- Linux with KVM (`/dev/kvm` accessible)
- Build with `--features hyperlight`

```bash
# Build with Hyperlight support
cargo build --features hyperlight

# Run Wasm modules (experimental)
agentkernel run --backend hyperlight module.wasm
```

**Key differences from Firecracker:**
- Runs WebAssembly modules only (not arbitrary shell commands)
- ~68ms startup vs 195ms daemon mode (2.9x faster)
- Sub-millisecond function calls after runtime is loaded
- Requires AOT-compiled Wasm modules for best performance

See [BENCHMARK.md](BENCHMARK.md) for detailed Hyperlight benchmarks.

**When to use daemon mode:**
- Running an API server
- Interactive development
- Many sequential commands
- Low latency requirements

**When to use ephemeral mode:**
- One-off commands
- Clean VM per execution
- Memory-constrained environments

## Examples

See the `examples/` directory for language-specific configurations:

```bash
./scripts/run-examples.sh     # Run all examples
```
