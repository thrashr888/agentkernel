# pi-agentkernel

Route [Pi coding agent](https://github.com/badlogic/pi-mono) commands through [agentkernel](https://github.com/thrashr888/agentkernel) microVM sandboxes.

## Install

```bash
# 1. Install and start agentkernel
brew tap thrashr888/agentkernel && brew install agentkernel
brew services start thrashr888/agentkernel/agentkernel

# 2. Install the extension
pi install npm:pi-agentkernel
```

## What it does

This extension overrides Pi's built-in `bash` tool so every shell command runs in an isolated sandbox instead of on your machine.

- **Session sandbox** — A persistent sandbox is created when Pi starts and removed when it ends
- **State persists** — Installed packages and files carry over between commands
- **Fallback** — If agentkernel isn't running, commands run locally

## Tools

| Tool | Description |
|------|-------------|
| `bash` | Built-in tool override — routes all shell commands through the sandbox |
| `sandbox_run` | One-shot command in a fresh sandbox (clean environment each time) |

## Commands

| Command | Description |
|---------|-------------|
| `/sandbox` | Show current sandbox status |

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `AGENTKERNEL_BASE_URL` | `http://localhost:18888` | agentkernel API endpoint |
| `AGENTKERNEL_API_KEY` | - | Optional Bearer token for API auth |

## Requirements

- [agentkernel](https://github.com/thrashr888/agentkernel) running locally or remotely
- Pi v0.49.0 or later

## License

Apache-2.0
