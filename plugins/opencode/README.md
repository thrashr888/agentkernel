# agentkernel OpenCode Plugin

Run OpenCode commands in hardware-isolated microVM sandboxes via agentkernel.

## Quick Start

**Option 1: Attach mode (recommended)**

OpenCode's server runs inside an agentkernel sandbox — full TUI, full isolation:

```bash
# Start agentkernel
agentkernel serve

# Connect OpenCode TUI to the sandboxed server
opencode attach http://localhost:18888/opencode
```

**Option 2: Plugin-based**

```bash
# Install the plugin into your project
agentkernel plugin install opencode

# Launch OpenCode — the plugin loads automatically
opencode
```

## Attach vs Plugin

| Feature | Attach Mode | Plugin |
|---------|------------|--------|
| Setup | Just `opencode attach <url>` | Install plugin files |
| Where OpenCode runs | Inside sandbox (isolated) | On your machine |
| Tools available | All OpenCode built-in tools | Adds sandbox tools |
| LLM API calls | From inside sandbox | From your machine |

## Setup

### 1. Install agentkernel

```bash
brew tap thrashr888/agentkernel && brew install agentkernel
# or
cargo install --git https://github.com/thrashr888/agentkernel
```

### 2. Start agentkernel

```bash
# As a background service (recommended)
brew services start thrashr888/agentkernel/agentkernel

# Or run manually
agentkernel serve
```

### 3. Launch OpenCode

```bash
# Attach mode (recommended)
opencode attach http://localhost:18888/opencode

# Or with plugin
agentkernel plugin install opencode
opencode
```

## Plugin Tools

When using the plugin, it adds three tools to OpenCode:

| Tool | Description |
|------|-------------|
| `sandbox_run` | One-shot command execution in a fresh sandbox |
| `sandbox_exec` | Run in the session's persistent sandbox (state persists) |
| `sandbox_list` | List all active sandboxes |

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENTKERNEL_BASE_URL` | `http://localhost:18888` | API endpoint |
| `AGENTKERNEL_API_KEY` | - | Optional Bearer token |

## How It Works

- **Attach mode**: OpenCode's server runs inside an agentkernel sandbox. All requests proxy through to it.
- **Plugin**: On `session.created`, a persistent sandbox is created. `sandbox_exec` runs commands in it.

Each sandbox runs in its own microVM with a dedicated Linux kernel — not a shared kernel like containers.

## License

MIT
