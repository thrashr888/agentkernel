
# Copilot CLI

Use [GitHub Copilot CLI](https://docs.github.com/en/copilot/concepts/agents/about-copilot-cli) with agentkernel for isolated code execution.

## Quick Start

Copilot CLI supports MCP natively. The agentkernel plugin registers MCP tools that Copilot CLI can call to run code in sandboxes.

```bash
# 1. Start agentkernel API server (pick one)
brew services start thrashr888/agentkernel/agentkernel   # runs in background, survives reboots
agentkernel serve                 # or run manually in a terminal

# 2. Install the plugin into your project
agentkernel plugin install copilot

# 3. Launch Copilot CLI — it picks up .mcp.json automatically
github-copilot
```

## MCP Integration

Copilot CLI runs on your machine and calls agentkernel MCP tools to execute code in isolated sandboxes. The plugin merges agentkernel's MCP server into your project's `.mcp.json`:

```bash
agentkernel plugin install copilot
```

For global installation (available in all projects):

```bash
agentkernel plugin install copilot --global
```

## API Key

Copilot CLI requires a GitHub token. Get one from [github.com/settings/tokens](https://github.com/settings/tokens).

Pass the token when running commands:

```bash
# Interactive session
agentkernel attach copilot-dev -e GITHUB_TOKEN=$GITHUB_TOKEN

# One-off command
agentkernel exec copilot-dev -e GITHUB_TOKEN=$GITHUB_TOKEN -- \
  github-copilot "Explain this code"
```

## Sandbox-Based Workflow

You can also run Copilot CLI itself inside an isolated sandbox container:

```bash
# Create sandbox with Copilot CLI pre-installed
agentkernel create copilot-dev --config examples/agents/copilot/agentkernel.toml

# Start the sandbox
agentkernel start copilot-dev

# Run Copilot CLI with your token
agentkernel attach copilot-dev -e GITHUB_TOKEN=$GITHUB_TOKEN

# Inside the sandbox:
github-copilot
```

## Configuration

Example config at `examples/agents/copilot/agentkernel.toml`:

```toml
[sandbox]
name = "copilot-sandbox"

[build]
dockerfile = "Dockerfile"

[agent]
preferred = "copilot"
compatibility_mode = "native"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"
network = true      # Copilot CLI needs network for GitHub API
mount_cwd = true    # Mount project directory
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENTKERNEL_BASE_URL` | `http://localhost:18888` | agentkernel API endpoint |
| `AGENTKERNEL_API_KEY` | - | Optional Bearer token for API auth |
| `GITHUB_TOKEN` | - | Required by Copilot CLI for authentication |

## What's Included

The sandbox image includes:

- **Node.js 22** — Runtime for Copilot CLI
- **Copilot CLI** — `@githubnext/github-copilot-cli`
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

# Copilot CLI
RUN npm install -g @githubnext/github-copilot-cli

# Your additions
RUN apk add --no-cache rust cargo

# Setup
WORKDIR /workspace
USER developer
```
