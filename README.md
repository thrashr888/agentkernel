# Agentkernel

Run AI coding agents in secure, isolated microVMs. Sub-125ms boot times, real hardware isolation.

## Installation

```bash
# macOS / Linux
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

Agentkernel automatically selects the right Docker image based on:

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
# Start the API server
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
  → Install Gemini CLI: pip install google-generativeai
Codex           installed       set
OpenCode        installed       set
```

## Why Agentkernel?

AI coding agents execute arbitrary code. Running them directly on your machine is risky:
- They can read/modify any file
- They can access your credentials and SSH keys
- Container escapes are a real threat

Agentkernel uses **Firecracker microVMs** (the same tech behind AWS Lambda) to provide true hardware isolation:

| Feature | Docker | Agentkernel |
|---------|--------|-------------|
| Isolation | Shared kernel | Separate kernel per VM |
| Boot time | 1-5 seconds | <125ms |
| Memory overhead | 50-100MB | <10MB |
| Escape risk | Container escapes possible | Hardware-enforced isolation |

## Platform Support

| Platform | Backend | Status |
|----------|---------|--------|
| Linux (x86_64, aarch64) | Firecracker microVMs | Full support |
| macOS (Apple Silicon, Intel) | Docker or Podman | Full support |

On macOS, agentkernel automatically falls back to containers since Firecracker requires KVM (Linux only). Podman is preferred if available (rootless, daemonless), otherwise Docker is used.

## Claude Code Integration

Agentkernel includes a Claude Code skill plugin for seamless AI agent integration.

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

## Benchmarks

Run your own benchmarks:

```bash
./scripts/benchmark.sh        # Latency per operation
./scripts/stress-test.sh 100 10  # Throughput (100 cmds, 10 concurrent)
```

### Docker Backend (macOS Apple Silicon)

**Latency per operation:**

| Operation | Avg | Min | Max |
|-----------|-----|-----|-----|
| Create | 52ms | 49ms | 58ms |
| Start | 235ms | 232ms | 240ms |
| Exec | 153ms | 104ms | 222ms |
| Stop | 126ms | 116ms | 132ms |
| Remove | 56ms | 48ms | 65ms |
| **Full Cycle** | **622ms** | - | - |

**Throughput (stress test):**

| Concurrency | Commands/sec | p50 Latency | p99 Latency |
|-------------|--------------|-------------|-------------|
| 1 | ~1.6 | 622ms | 650ms |
| 5 | ~10 | 1,656ms | 1,893ms |
| 10 | ~9 | 4,500ms | 5,208ms |

*Results from Docker via Colima on M1 MacBook Pro. Firecracker on Linux will be significantly faster.*

## Examples

See the `examples/` directory for language-specific configurations:

```bash
./scripts/run-examples.sh     # Run all examples
./scripts/benchmark.sh        # Latency benchmark
./scripts/stress-test.sh      # Throughput benchmark
```

## Roadmap

- [x] MCP server for programmatic integration
- [x] HTTP API for external agents
- [x] Permission/restriction profiles
- [x] Multi-agent support (Claude, Gemini, Codex, OpenCode)
- [ ] macOS Seatbelt backend (lightweight, no containers)
- [ ] Filesystem mounting and syncing
- [ ] Native Firecracker microVMs (Linux)
