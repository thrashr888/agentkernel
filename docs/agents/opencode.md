
# OpenCode

Run [OpenCode](https://opencode.ai/) with agentkernel for isolated code execution.

## Quick Start — `opencode attach` (Recommended)

The fastest way to use OpenCode with agentkernel. OpenCode's full server runs inside an isolated sandbox, and you connect to it with `attach`:

```bash
# 1. Start agentkernel
agentkernel serve

# 2. Connect OpenCode TUI — sandbox is created automatically
opencode attach http://localhost:18888/opencode
```

On first connect, agentkernel creates a sandbox from the `opencode-sandbox` template, installs OpenCode inside it, and starts `opencode serve`. All subsequent requests proxy through to the OpenCode server in the sandbox. First connect takes ~8s; after that it's instant.

### How It Works

1. `opencode attach` connects to agentkernel's `/opencode` proxy
2. agentkernel provisions a sandbox using the built-in `opencode-sandbox` template
3. OpenCode's own server (`opencode serve`) runs inside the sandbox on port 3000
4. All OpenCode protocol requests (sessions, messages, SSE events) proxy through to it
5. You get the full OpenCode TUI experience with sandbox isolation

## Alternative: Plugin Integration

For users who prefer plugin-based integration, agentkernel also provides an OpenCode plugin that adds sandbox tools to OpenCode:

```bash
# Install the plugin into your project
agentkernel plugin install opencode

# Launch OpenCode — the plugin loads automatically
opencode
```

The plugin adds tools to OpenCode:

| Tool | Description |
|------|-------------|
| `sandbox_run` | One-shot command in a fresh sandbox |
| `sandbox_exec` | Run in the session's persistent sandbox (state persists) |
| `sandbox_list` | List all active sandboxes |

## Alternative: Manual Sandbox

You can also create an OpenCode sandbox manually using the template or Dockerfile:

```bash
# Using the built-in template
agentkernel sandbox create opencode-dev --template opencode-sandbox

# Or using the example Dockerfile (pre-built image)
docker build -t agentkernel/opencode examples/agents/opencode/
agentkernel sandbox create opencode-dev --config examples/agents/opencode/agentkernel.toml

# Attach to the sandbox and run OpenCode directly
agentkernel attach opencode-dev
# Inside the sandbox:
opencode
```

## Setup

### 1. Install agentkernel

```bash
brew tap thrashr888/agentkernel && brew install agentkernel
# Or: curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
```

### 2. Start agentkernel

```bash
# As a background service (recommended — survives reboots)
brew services start thrashr888/agentkernel/agentkernel

# Or run manually
agentkernel serve
```

### 3. Connect OpenCode

```bash
# Attach mode (recommended — full TUI, sandboxed)
opencode attach http://localhost:18888/opencode

# Or with plugin
agentkernel plugin install opencode
opencode
```

## Configuration

The built-in `opencode-sandbox` template:

```toml
[sandbox]
name = "opencode-sandbox"
base_image = "node:22-alpine"
init_script = """
set -e
apk add --no-cache git bash curl python3 ripgrep fd jq
curl -fsSL https://opencode.ai/install | bash
export PATH="$HOME/.opencode/bin:$PATH"
"""

[agent]
preferred = "opencode"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"

[security.domains]
allow = ["api.openai.com", "api.anthropic.com"]
```

For custom setups, see `examples/agents/opencode/agentkernel.toml` and the Dockerfile.

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENTKERNEL_BASE_URL` | `http://localhost:18888` | agentkernel API endpoint |
| `AGENTKERNEL_API_KEY` | - | Optional Bearer token for API auth |

OpenCode itself supports multiple LLM providers. Pass your provider's API key as usual (e.g. `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`).

## What's Included

The sandbox comes with:

- **Node.js 22** — Runtime
- **OpenCode CLI** — installed via `opencode.ai/install`
- **Git** — Version control
- **Python 3** — For Python projects
- **ripgrep** — Fast code search
- **fd** — Fast file finder
- **jq** — JSON processing
