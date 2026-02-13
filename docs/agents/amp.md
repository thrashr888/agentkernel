
# Amp

Use [Amp](https://ampcode.com/) (by Sourcegraph) with agentkernel for isolated code execution.

## Quick Start

Amp supports MCP natively. The agentkernel plugin registers MCP tools that Amp can call to run code in sandboxes.

```bash
# 1. Start agentkernel API server (pick one)
brew services start thrashr888/agentkernel/agentkernel   # runs in background, survives reboots
agentkernel serve                 # or run manually in a terminal

# 2. Install the plugin into your project
agentkernel plugin install amp

# 3. Launch Amp — it picks up .mcp.json automatically
amp
```

## MCP Integration

Amp runs on your machine and calls agentkernel MCP tools to execute code in isolated sandboxes. The plugin merges agentkernel's MCP server into your project's `.mcp.json`:

```bash
agentkernel plugin install amp
```

For global installation (available in all projects):

```bash
agentkernel plugin install amp --global
```

## API Key

Amp uses an Anthropic API key. Get one from [console.anthropic.com](https://console.anthropic.com/).

**Important:** Use a regular API key (`sk-ant-api03-...`), not an OAuth token.

## Sandbox-Based Workflow

You can also run Amp itself inside an isolated sandbox container:

```bash
# Create sandbox with Amp pre-installed
agentkernel create amp-dev --config examples/agents/amp/agentkernel.toml

# Start the sandbox
agentkernel start amp-dev

# Run Amp with your API key
agentkernel attach amp-dev -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY

# Inside the sandbox:
amp
```

## Configuration

Example config at `examples/agents/amp/agentkernel.toml`:

```toml
[sandbox]
name = "amp-sandbox"

[build]
dockerfile = "Dockerfile"

[agent]
preferred = "amp"
compatibility_mode = "amp"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"
network = true      # Amp needs network for Anthropic API + Sourcegraph
mount_cwd = true    # Mount project directory
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENTKERNEL_BASE_URL` | `http://localhost:18888` | agentkernel API endpoint |
| `AGENTKERNEL_API_KEY` | - | Optional Bearer token for API auth |
| `ANTHROPIC_API_KEY` | - | Required by Amp for LLM calls |

## What's Included

The sandbox image includes:

- **Node.js 22** — Runtime for Amp
- **Amp CLI** — `@sourcegraph/amp`
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

# Amp CLI
RUN npm install -g @sourcegraph/amp

# Your additions
RUN apk add --no-cache rust cargo

# Setup
WORKDIR /workspace
USER developer
```
