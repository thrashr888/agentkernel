# agentkernel Pi Extension

Route Pi coding agent commands through hardware-isolated microVM sandboxes via agentkernel.

## Setup

1. Install agentkernel:

```bash
brew tap thrashr888/agentkernel && brew install agentkernel
# or
cargo install --git https://github.com/thrashr888/agentkernel
```

2. Install the extension into your project:

```bash
agentkernel plugin install pi
```

3. Start agentkernel:

```bash
# As a background service (recommended)
brew services start thrashr888/agentkernel/agentkernel

# Or run manually
agentkernel serve
```

4. Launch Pi — the extension loads automatically.

## What It Does

The extension overrides Pi's built-in `bash` tool. Every shell command the LLM runs goes through an agentkernel sandbox instead of running on your machine.

| Tool | Description |
|------|-------------|
| `bash` | Built-in tool override — routes all shell commands through the sandbox |
| `sandbox_run` | One-shot command in a fresh sandbox (clean environment each time) |

| Command | Description |
|---------|-------------|
| `/sandbox` | Show current sandbox status |

## How It Works

- On `session_start`: a persistent sandbox is created for the session
- `bash` tool: commands run in the session sandbox (packages/files persist)
- `sandbox_run`: runs one-shot commands in fresh sandboxes
- On `session_shutdown`: the session sandbox is automatically removed
- If agentkernel is not running, bash falls back to local execution

Each sandbox runs in its own microVM with a dedicated Linux kernel.

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENTKERNEL_BASE_URL` | `http://localhost:18888` | API endpoint |
| `AGENTKERNEL_API_KEY` | - | Optional Bearer token |

## License

MIT
