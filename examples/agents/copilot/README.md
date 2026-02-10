# Copilot CLI Agent Image

A Docker image with GitHub Copilot CLI pre-installed for use with agentkernel.

## Quick Start

```bash
# Create a sandbox (builds the image automatically)
agentkernel create my-project --config agentkernel.toml --dir /path/to/your/project

# Start and attach
agentkernel start my-project
agentkernel attach my-project

# Inside the sandbox, run Copilot CLI
github-copilot
```

The Dockerfile is built automatically when you use the config file.

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `GITHUB_TOKEN` | Yes | Your GitHub token for Copilot |

## Passing API Keys

When creating a sandbox, pass your token:

```bash
# Option 1: Set in environment before creating
export GITHUB_TOKEN=ghp_...
agentkernel create my-project --image agentkernel/copilot

# Option 2: Pass via exec
agentkernel exec my-project -- env GITHUB_TOKEN=$GITHUB_TOKEN github-copilot
```

## What's Included

- **Node.js 22** - Runtime for Copilot CLI
- **Copilot CLI** - `@githubnext/github-copilot-cli`
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast search
- **fd** - Fast file finder
- **jq** - JSON processing
- **bash** - Shell

## Security Notes

- Runs as non-root user `developer`
- Workspace is isolated at `/workspace`
- Network access is controlled by agentkernel security profiles
