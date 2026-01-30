# agentkernel OpenCode Plugin

Run OpenCode commands in hardware-isolated microVM sandboxes via agentkernel.

## Setup

1. Install agentkernel:

```bash
brew tap thrashr888/agentkernel && brew install agentkernel
# or
cargo install --git https://github.com/thrashr888/agentkernel
```

2. Install the plugin into your project:

```bash
agentkernel plugin install opencode
```

3. Start agentkernel:

```bash
# As a background service (recommended)
brew services start agentkernel

# Or run manually
agentkernel serve
```

4. Launch OpenCode — the plugin loads automatically.

## Tools

The plugin adds three tools to OpenCode:

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

- On `session.created`: a persistent sandbox is created for the session
- `sandbox_exec`: runs commands in the session sandbox (packages/files persist)
- `sandbox_run`: runs one-shot commands in fresh sandboxes
- On `session.deleted`: the session sandbox is automatically removed

Each sandbox runs in its own microVM with a dedicated Linux kernel — not a shared kernel like containers.

## License

MIT
