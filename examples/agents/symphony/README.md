# Symphony Image

A Docker image with OpenAI Symphony orchestration daemon pre-installed for use with agentkernel.

## Quick Start

```bash
# Create a sandbox (builds the image automatically)
agentkernel sandbox create my-project --config agentkernel.toml --dir /path/to/your/project

# Start and attach
agentkernel sandbox start my-project
agentkernel attach my-project

# Inside the sandbox, create a WORKFLOW.md and run Symphony
cd /opt/symphony/elixir && mix run -- /workspace/WORKFLOW.md --port 4000
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `LINEAR_API_KEY` | Yes | Linear API key for issue tracking |
| `OPENAI_API_KEY` | Yes | OpenAI API key for Codex agent |

## What's Included

- **Elixir 1.19** with **OTP 28** - Runtime for Symphony
- **Node.js 22** - For Codex CLI
- **Codex CLI** - `@openai/codex` (the coding agent Symphony spawns)
- **Git** - Version control
- **bash** - Shell

## Security Notes

- Runs as non-root user `developer`
- Workspace is isolated at `/workspace`
- Network access is controlled by agentkernel security profiles
- Dashboard available on port 4000 when started with `--port 4000`
