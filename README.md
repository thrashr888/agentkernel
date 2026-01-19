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
# In your project directory
agentkernel run

# That's it. Your AI agent now runs in an isolated microVM.
```

Agentkernel automatically:
- Detects your project type (Python, Node, Go, Rust)
- Boots a microVM with the right runtime (~125ms)
- Mounts your project files
- Runs your preferred AI agent (Claude Code, Gemini CLI, Codex, etc.)

## Usage

```bash
# Run with default settings (auto-detect everything)
agentkernel run

# Specify an agent
agentkernel run --agent claude
agentkernel run --agent gemini
agentkernel run --agent codex

# Run a specific command in the sandbox
agentkernel exec "npm test"
agentkernel exec "python -m pytest"

# Interactive shell inside the microVM
agentkernel shell
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
