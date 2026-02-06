# agentkernel OpenCode Plugin

Run OpenCode commands in hardware-isolated microVM sandboxes via agentkernel.

## Quick Start

**Option 1: Native API (recommended)**

agentkernel implements OpenCode's native HTTP API — no plugin needed:

```bash
# Start agentkernel
agentkernel serve

# Connect OpenCode directly
opencode --api-url http://localhost:18888/opencode
```

**Option 2: Plugin-based**

```bash
# Install the plugin into your project
agentkernel plugin install opencode

# Launch OpenCode — the plugin loads automatically
opencode
```

## Native API vs Plugin

| Feature | Native API | Plugin |
|---------|-----------|--------|
| Setup | Just `--api-url` flag | Install plugin files |
| Session management | Automatic | Automatic |
| Tool discovery | OpenCode's built-in tools | Adds custom tools |
| Portability | Works anywhere | Per-project |

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
# Native API (recommended)
opencode --api-url http://localhost:18888/opencode

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

- **Native API**: Sessions map directly to agentkernel sandboxes. Messages are executed as shell commands.
- **Plugin**: On `session.created`, a persistent sandbox is created. `sandbox_exec` runs commands in it.

Each sandbox runs in its own microVM with a dedicated Linux kernel — not a shared kernel like containers.

## License

MIT
