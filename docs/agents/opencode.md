
# OpenCode

Run [OpenCode](https://opencode.ai/) with agentkernel as the execution backend.

## Quick Start

agentkernel implements OpenCode's native HTTP API, allowing OpenCode to connect directly without plugins.

```bash
# 1. Start agentkernel API server (pick one)
brew services start thrashr888/agentkernel/agentkernel   # runs in background, survives reboots
agentkernel serve                 # or run manually in a terminal

# 2. Launch OpenCode with agentkernel as the backend
opencode --api-url http://localhost:18888/opencode
```

## Native API Integration

agentkernel implements OpenCode's server API at the `/opencode` path prefix. This provides seamless integration without any plugins or configuration files.

### Endpoint Status

| Endpoint | Status | Description |
|----------|--------|-------------|
| `GET /opencode/session` | ✓ | List all sessions |
| `POST /opencode/session` | ✓ | Create a new session (creates sandbox) |
| `GET /opencode/session/{id}` | ✓ | Get session details |
| `POST /opencode/session/{id}/message` | ✓ | Execute command in sandbox |
| `GET /opencode/session/{id}/message` | ✓ | Get message history |
| `GET /opencode/event` | ✓ | SSE stream for session events |
| `GET /opencode/global/event` | ✓ | SSE stream for global events |
| `GET /opencode/permission` | ✓ | List pending permissions (auto-approved) |
| `POST /opencode/permission/{id}/reply` | ✓ | Reply to permission |
| `GET /opencode/question` | ✓ | List pending questions (none) |
| `POST /opencode/question/{id}/reply` | ✓ | Reply to question |
| `GET /opencode/provider` | − | Stub (returns empty) |
| `GET /opencode/agent` | − | Stub |
| `GET /opencode/config` | − | Stub |

### How It Works

1. **Session = Sandbox**: Each OpenCode session maps to an agentkernel sandbox
2. **Messages = Commands**: Sending a message executes it as a shell command in the sandbox
3. **State persists**: Installed packages and files persist between commands within a session
4. **Auto-approval**: All tool permissions are auto-approved (sandboxed execution is safe)

## Alternative: Plugin Integration

For users who prefer plugin-based integration, agentkernel also provides an OpenCode plugin.

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
agentkernel serve --host 127.0.0.1 --port 18888
```

### 3. Launch OpenCode

```bash
# Native API (recommended)
opencode --api-url http://localhost:18888/opencode

# Or with plugin
agentkernel plugin install opencode
opencode
```

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

Example config at `examples/agents/opencode/agentkernel.toml`:

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
