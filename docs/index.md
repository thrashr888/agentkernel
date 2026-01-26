---
title: Home
permalink: /index.html
sidebar: agentkernel_sidebar
topnav: topnav
---

# agentkernel

Run AI coding agents in secure, isolated microVMs. Sub-125ms boot times, real hardware isolation.

## What is agentkernel?

agentkernel is a sandbox runtime for AI coding agents. It provides:

- **True isolation** - Each sandbox runs in its own microVM with a dedicated kernel
- **Fast startup** - Sub-125ms boot times with pre-warmed VM pools
- **Multi-agent support** - Run Claude Code, Codex, Gemini CLI, and more
- **Simple CLI** - Familiar Docker-like commands

## Quick Start

```bash
# Install
curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
agentkernel setup

# Run a command in isolation
agentkernel run python3 -c "print('Hello from sandbox!')"

# Create a persistent sandbox with Claude Code
agentkernel create my-project --config examples/agents/claude-code/agentkernel.toml
agentkernel start my-project
agentkernel attach my-project -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY
```

## Why agentkernel?

AI coding agents need to execute arbitrary code. Running them directly on your machine is risky. Docker provides some isolation, but shares the host kernel. agentkernel provides **real hardware isolation** via microVMs while keeping startup times fast.

| Feature | Docker | agentkernel |
|---------|--------|-------------|
| Kernel isolation | Shared | Dedicated |
| Boot time | ~200ms | <125ms |
| Memory overhead | ~50MB | ~10MB |
| Security | Container escape possible | Hardware-enforced |

## Supported Platforms

| Platform | Backend | Status |
|----------|---------|--------|
| Linux | Firecracker (KVM) | Full support |
| Linux | Docker | Full support |
| Linux | Podman | Full support |
| macOS | Docker Desktop | Full support |
| macOS | Podman | Full support |
| macOS 26+ | Apple Containers | Beta |
| Windows | WSL2 + Docker | Untested |

## Next Steps

- [Installation](installation.html) - Detailed setup instructions
- [Getting Started](getting-started.html) - Your first sandbox
- [Commands](commands.html) - CLI reference
- [Configuration](configuration.html) - Config file format
