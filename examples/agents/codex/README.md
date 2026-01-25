# OpenAI Codex Agent Image

A Docker image with OpenAI Codex CLI pre-installed for use with agentkernel.

## Quick Start

```bash
# Build the image
docker build -t agentkernel/codex .

# Create a sandbox with this image
agentkernel create my-project --config agentkernel.toml

# Start and attach
agentkernel start my-project
agentkernel attach my-project

# Inside the sandbox, run Codex
codex
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `OPENAI_API_KEY` | Yes | Your OpenAI API key |

## What's Included

- **Node.js 22** - Runtime for Codex
- **Codex CLI** - `@openai/codex`
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast search
- **fd** - Fast file finder
- **bash** - Shell

## Security Notes

- Runs as non-root user `developer`
- Workspace is isolated at `/workspace`
- Network access is controlled by agentkernel security profiles
