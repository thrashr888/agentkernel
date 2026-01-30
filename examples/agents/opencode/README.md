# OpenCode Agent Image

A Docker image with OpenCode CLI pre-installed for use with agentkernel.

## Quick Start

```bash
# Build the image
docker build -t agentkernel/opencode .

# Create a sandbox with this image
agentkernel create my-project --config agentkernel.toml

# Start and attach
agentkernel start my-project
agentkernel attach my-project

# Inside the sandbox, run OpenCode
opencode
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| Provider API key | Yes | Your LLM provider's key (e.g. `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`) |

OpenCode supports multiple LLM providers. Pass whichever key your provider requires.

## What's Included

- **Node.js 22** — Runtime for OpenCode
- **OpenCode CLI** — `opencode`
- **Git** — Version control
- **Python 3** — For Python projects
- **ripgrep** — Fast search
- **fd** — Fast file finder
- **bash** — Shell

## Security Notes

- Runs as non-root user `developer`
- Workspace is isolated at `/workspace`
- Network access is controlled by agentkernel security profiles
