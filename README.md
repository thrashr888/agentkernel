# Agentkernel

Run AI coding agents in secure, isolated microVMs. Sub-125ms boot times, real hardware isolation.

## Installation

```bash
# macOS / Linux
curl -fsSL https://agentkernel.dev/install.sh | sh

# Or with Cargo
cargo install agentkernel
```

## Quick Start

```bash
# Initialize config in your project
agentkernel init

# Create and start a sandbox
agentkernel create my-project --dir .
agentkernel start my-project

# Open interactive shell in the sandbox
agentkernel attach my-project
```

## Usage

```bash
# Create a sandbox with a specific agent
agentkernel create my-project --agent claude --dir .
agentkernel create my-project --agent gemini --dir .

# Start and attach
agentkernel start my-project
agentkernel attach my-project

# Run a specific command
agentkernel exec my-project npm test
agentkernel exec my-project python -m pytest

# List running sandboxes
agentkernel list

# Stop and remove
agentkernel stop my-project
agentkernel remove my-project
```

## Configuration

Create `agentkernel.toml` in your project root for custom settings:

```toml
[sandbox]
runtime = "python"      # python, node, go, rust, or base

[agent]
preferred = "claude"    # claude, gemini, codex, opencode

[resources]
vcpus = 2
memory_mb = 512
```

Most projects don't need a config file - agentkernel auto-detects everything.

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

| Platform | Status |
|----------|--------|
| Linux (x86_64, aarch64) | Supported |
| macOS (Apple Silicon, Intel) | Supported via Docker Desktop |

## License

MIT
