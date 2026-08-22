# Hermes Agent Image

A Docker image with Hermes Agent (NousResearch) pre-installed for use with agentkernel.

## Quick Start

```bash
# Create a sandbox (builds the image automatically)
agentkernel sandbox create my-project --config agentkernel.toml --dir /path/to/your/project

# Start and attach
agentkernel sandbox start my-project
agentkernel attach my-project

# Inside the sandbox, run Hermes
cd /opt/hermes-agent && python -m hermes_cli.main
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `OPENROUTER_API_KEY` | Yes (one of) | OpenRouter API key (recommended, access to all models) |
| `ANTHROPIC_API_KEY` | Yes (one of) | Anthropic API key (direct access) |

Hermes supports multiple LLM providers via LiteLLM. Provide at least one API key.

## What's Included

- **Python 3.11** - Runtime for Hermes
- **Node.js 24 LTS** - For browser automation tools
- **Hermes Agent** - Full install from source with all tools
- **mini-swe-agent** - Terminal tool backend
- **Git** - Version control
- **ripgrep** - Fast search
- **bash** - Shell

## Security Notes

- Runs as non-root user `developer`
- Workspace is isolated at `/workspace`
- Network access is controlled by agentkernel security profiles
