# Google Gemini CLI Agent Image

A Docker image with Google Gemini CLI pre-installed for use with agentkernel.

## Quick Start

```bash
# Build the image
docker build -t agentkernel/gemini .

# Create a sandbox with this image
agentkernel create my-project --config agentkernel.toml

# Start and attach
agentkernel start my-project
agentkernel attach my-project

# Inside the sandbox, run Gemini
gemini
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `GEMINI_API_KEY` | Yes | Your Google AI API key |

## What's Included

- **Node.js 22** - Runtime for Gemini CLI
- **Gemini CLI** - `@google/gemini-cli`
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast search
- **fd** - Fast file finder
- **bash** - Shell

## Security Notes

- Runs as non-root user `developer`
- Workspace is isolated at `/workspace`
- Network access is controlled by agentkernel security profiles
