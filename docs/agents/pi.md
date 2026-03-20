
# Pi

Run [pi-coding-agent](https://github.com/badlogic/pi-mono) with agentkernel for isolated code execution.

## Quick Start

Pi is an open-source coding agent with its own extension system. The agentkernel extension overrides Pi's built-in `bash` tool so every shell command runs in a microVM sandbox instead of on your machine.

```bash
# 1. Start agentkernel API server (pick one)
brew services start thrashr888/agentkernel/agentkernel   # runs in background, survives reboots
agentkernel serve                 # or run manually in a terminal

# 2. Install the extension (pick one)
pi install npm:pi-agentkernel     # via Pi's package manager (recommended)
agentkernel plugin install pi     # or via agentkernel CLI

# 3. Launch Pi — the extension loads automatically
pi
```

## Extension Integration

Unlike MCP-based agents, Pi uses a native extension that replaces the built-in `bash` tool. When the extension loads:

1. A persistent sandbox is created for the session
2. Every `bash` call the LLM makes runs inside that sandbox via `POST /sandboxes/{name}/exec`
3. State persists between calls — installed packages and files carry over
4. The sandbox is removed when the session ends

If agentkernel is not running, the extension falls back to local execution.

The extension also provides:

| Tool | Description |
|------|-------------|
| `bash` | Built-in tool override — routes all shell commands through the sandbox |
| `sandbox_run` | One-shot command in a fresh sandbox (clean environment each time) |

| Command | Description |
|---------|-------------|
| `/sandbox` | Show current sandbox status |

### Install

**Via Pi's package manager (recommended):**
```bash
pi install npm:pi-agentkernel
```

**Or via agentkernel CLI:**
```bash
agentkernel plugin install pi
```

The first method installs globally. The second creates `.pi/extensions/agentkernel/` in your project.

## Setup

### 1. Install agentkernel

```bash
brew tap thrashr888/tap && brew install agentkernel
# Or: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
```

### 2. Install the extension

```bash
agentkernel plugin install pi
```

Your project should have:

```
.pi/
  extensions/
    agentkernel/
      index.ts          # Extension source
      package.json      # Metadata
```

### 3. Start agentkernel

```bash
# As a background service (recommended — survives reboots)
brew services start thrashr888/agentkernel/agentkernel

# Or run manually
agentkernel serve --host 127.0.0.1 --port 18888
```

### 4. Launch Pi

```bash
pi
```

The extension loads automatically and notifies when the sandbox is ready.

## API Keys

Pi supports multiple LLM providers. Pass your provider's API key as usual — it stays on your machine and is not forwarded to the sandbox:

```bash
# Anthropic
export ANTHROPIC_API_KEY=sk-ant-...

# OpenAI
export OPENAI_API_KEY=sk-...

# Google
export GOOGLE_API_KEY=AI...
```

## Sandbox-Based Workflow

You can also run Pi itself inside an isolated sandbox container:

```bash
# Create sandbox with Pi pre-installed
agentkernel sandbox create pi-dev --config examples/agents/pi/agentkernel.toml

# Start the sandbox
agentkernel sandbox start pi-dev

# Run Pi inside the sandbox
agentkernel attach pi-dev
# Inside the sandbox:
pi
```

## Configuration

Example config at `examples/agents/pi/agentkernel.toml`:

```toml
[sandbox]
name = "pi-sandbox"

[build]
dockerfile = "Dockerfile"

[agent]
preferred = "pi"
compatibility_mode = "pi"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"
network = true      # Pi needs network for LLM API calls
mount_cwd = true    # Mount project directory
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENTKERNEL_BASE_URL` | `http://localhost:18888` | agentkernel API endpoint |
| `AGENTKERNEL_API_KEY` | - | Optional Bearer token for API auth |

Pi itself supports multiple LLM providers. Pass your provider's API key as usual — it stays on your machine and is not forwarded to the sandbox.

## What's Included

The sandbox image includes:

- **Node.js 22** — Runtime for Pi
- **Pi CLI** — `@mariozechner/pi-coding-agent`
- **Git** — Version control
- **Python 3** — For Python projects
- **ripgrep** — Fast code search
- **fd** — Fast file finder
- **jq** — JSON processing

## Customizing

Create a custom Dockerfile based on the example:

```dockerfile
FROM node:22-alpine

# Base tools
RUN apk add --no-cache git bash python3 ripgrep fd jq

# Pi CLI
RUN npm install -g @mariozechner/pi-coding-agent

# Your additions
RUN apk add --no-cache rust cargo

# Setup
WORKDIR /workspace
USER developer
```
