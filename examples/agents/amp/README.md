# Amp Agent Image

A Docker image with Amp (Sourcegraph) CLI pre-installed for use with agentkernel.

## Quick Start

```bash
# Create a sandbox (builds the image automatically)
agentkernel sandbox create my-project --config agentkernel.toml --dir /path/to/your/project

# Start and attach
agentkernel sandbox start my-project
agentkernel attach my-project

# Inside the sandbox, run Amp
amp
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `ANTHROPIC_API_KEY` | Yes | Your Anthropic API key for Amp |

## What's Included

- **Node.js 24 LTS** - Runtime for Amp
- **Amp CLI** - `@ampcode/cli`
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast search
- **fd** - Fast file finder
- **bash** - Shell

## Security Notes

- Runs as non-root user `developer`
- Workspace is isolated at `/workspace`
- Network access is controlled by agentkernel security profiles
