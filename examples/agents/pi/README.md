# Pi Agent Image

A Docker image with Pi (Mario Zechner's coding agent) pre-installed for use with agentkernel.

## Quick Start

```bash
# Create a sandbox (builds the image automatically)
agentkernel create my-project --config agentkernel.toml --dir /path/to/your/project

# Start and attach
agentkernel start my-project
agentkernel attach my-project

# Inside the sandbox, run Pi
pi
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `ANTHROPIC_API_KEY` | Yes (one of) | Anthropic API key |
| `OPENAI_API_KEY` | Yes (one of) | OpenAI API key |

Pi supports multiple LLM providers. Provide at least one API key.

## What's Included

- **Node.js 22** - Runtime for Pi
- **Pi CLI** - `@mariozechner/pi-coding-agent`
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast search
- **fd** - Fast file finder
- **bash** - Shell

## Security Notes

- Runs as non-root user `developer`
- Workspace is isolated at `/workspace`
- Network access is controlled by agentkernel security profiles
