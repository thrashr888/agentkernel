
# OpenCode

Run [OpenCode](https://opencode.ai/) in an isolated sandbox.

## Quick Start

OpenCode integrates with agentkernel through a TypeScript plugin that automatically sandboxes code execution.

```bash
# 1. Start agentkernel API server (pick one)
brew services start thrashr888/agentkernel/agentkernel   # runs in background, survives reboots
agentkernel serve                 # or run manually in a terminal

# 2. Install the plugin into your project
agentkernel plugin install opencode

# 3. Launch OpenCode — the plugin loads automatically
opencode
```

## Plugin Integration

Unlike other agents that run inside a sandbox container, OpenCode runs on your machine and delegates execution to agentkernel via the HTTP API. The plugin adds three tools to OpenCode:

| Tool | Description |
|------|-------------|
| `sandbox_run` | One-shot command in a fresh sandbox |
| `sandbox_exec` | Run in the session's persistent sandbox (state persists) |
| `sandbox_list` | List all active sandboxes |

When a session starts, the plugin creates a persistent sandbox. Commands via `sandbox_exec` run inside it, so installed packages and files persist between calls. The sandbox is automatically removed when the session ends.

## Setup

### 1. Install agentkernel

```bash
brew tap thrashr888/agentkernel && brew install agentkernel
# Or: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
```

### 2. Install the plugin

Install the OpenCode plugin files into your project:

```bash
agentkernel plugin install opencode
```

This creates the `.opencode/` directory with `package.json` and `plugins/agentkernel.ts`.

Your project should have:

```
.opencode/
  package.json              # Plugin dependency
  plugins/
    agentkernel.ts          # Plugin source
```

### 3. Start agentkernel

```bash
# As a background service (recommended — survives reboots)
brew services start thrashr888/agentkernel/agentkernel

# Or run manually
agentkernel serve --host 127.0.0.1 --port 18888
```

### 4. Launch OpenCode

```bash
opencode
```

The plugin loads automatically and logs `agentkernel plugin loaded` on startup.

## Sandbox-Based Workflow

You can also run OpenCode itself inside a sandbox container:

```bash
# Create sandbox with OpenCode pre-installed
agentkernel create opencode-dev --config examples/agents/opencode/agentkernel.toml

# Start the sandbox
agentkernel start opencode-dev

# Run OpenCode inside the sandbox
agentkernel attach opencode-dev
# Inside the sandbox:
opencode
```

## Configuration

The example config at `examples/agents/opencode/agentkernel.toml`:

```toml
[sandbox]
name = "opencode-sandbox"

[build]
dockerfile = "Dockerfile"

[agent]
preferred = "opencode"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"
network = true      # OpenCode needs network for API calls
mount_cwd = true    # Mount project directory
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENTKERNEL_BASE_URL` | `http://localhost:18888` | agentkernel API endpoint |
| `AGENTKERNEL_API_KEY` | - | Optional Bearer token for API auth |

OpenCode itself supports multiple LLM providers. Pass your provider's API key as usual — it stays on your machine and is not forwarded to the sandbox.

## What's Included

The sandbox image includes:

- **Node.js 22** — Runtime
- **OpenCode CLI** — `opencode`
- **Git** — Version control
- **Python 3** — For Python projects
- **ripgrep** — Fast code search
- **fd** — Fast file finder
- **jq** — JSON processing
