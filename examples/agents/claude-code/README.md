# Claude Code Agent Image

A Docker image with Claude Code CLI pre-installed for use with agentkernel.

## Quick Start

```bash
# Create a sandbox (builds the image automatically)
agentkernel create my-project --config agentkernel.toml --dir /path/to/your/project

# Start and attach
agentkernel start my-project
agentkernel attach my-project

# Inside the sandbox, run Claude Code
claude
```

The Dockerfile is built automatically when you use the config file.

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `ANTHROPIC_API_KEY` | Yes | Your Anthropic API key for Claude |

## Passing API Keys

When creating a sandbox, pass your API key:

```bash
# Option 1: Set in environment before creating
export ANTHROPIC_API_KEY=sk-ant-...
agentkernel create my-project --image agentkernel/claude-code

# Option 2: Pass via exec
agentkernel exec my-project -- env ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY claude
```

## What's Included

- **Node.js 22** - Runtime for Claude Code
- **Claude Code CLI** - `@anthropic-ai/claude-code`
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast search (used by Claude Code)
- **fd** - Fast file finder
- **jq** - JSON processing
- **bash** - Shell

## Customizing

Create your own Dockerfile based on this one:

```dockerfile
FROM agentkernel/claude-code

# Add your tools
RUN apk add --no-cache your-tools

# Add project-specific setup
COPY your-config /home/developer/.config/
```

## Security Notes

- Runs as non-root user `developer`
- Workspace is isolated at `/workspace`
- Network access is controlled by agentkernel security profiles
