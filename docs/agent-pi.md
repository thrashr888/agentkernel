
# Pi

Run [pi-coding-agent](https://github.com/badlogic/pi-mono) with agentkernel for isolated code execution.

## Quick Start

Pi is an open-source coding agent that supports MCP. The agentkernel plugin gives Pi access to sandbox tools for isolated code execution.

```bash
# 1. Start agentkernel API server (pick one)
brew services start thrashr888/agentkernel/agentkernel   # runs in background, survives reboots
agentkernel serve                 # or run manually in a terminal

# 2. Install the plugin into your project
agentkernel plugin install pi

# 3. Launch Pi — it picks up .mcp.json automatically
pi
```

## Plugin Integration

Pi runs on your machine and delegates code execution to agentkernel via MCP tools. The plugin merges agentkernel's MCP server into your project's `.mcp.json`:

```bash
agentkernel plugin install pi
```

For global installation (available in all projects):

```bash
agentkernel plugin install pi --global
```

Because Pi is open source, deeper integration is possible — a future plugin could swap Pi's built-in code runner for agentkernel sandboxes entirely, similar to the [OpenCode plugin](agent-opencode.md).

## Setup

### 1. Install agentkernel

```bash
brew tap thrashr888/agentkernel && brew install agentkernel
# Or: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
```

### 2. Install the plugin

```bash
agentkernel plugin install pi
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

## API Keys

Pi supports multiple LLM providers. Pass your provider's API key as usual — it stays on your machine and is not forwarded to the sandbox unless you explicitly pass it:

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
agentkernel create pi-dev --config examples/agents/pi/agentkernel.toml

# Start the sandbox
agentkernel start pi-dev

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
