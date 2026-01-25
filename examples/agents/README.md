# Agent Images for Agentkernel

Pre-built Docker images for running AI coding agents in isolated sandboxes.

## Available Agents

| Agent | Directory | Image | API Key Env |
|-------|-----------|-------|-------------|
| Claude Code | `claude-code/` | `agentkernel/claude-code` | `ANTHROPIC_API_KEY` |
| OpenAI Codex | `codex/` | `agentkernel/codex` | `OPENAI_API_KEY` |
| Google Gemini | `gemini/` | `agentkernel/gemini` | `GEMINI_API_KEY` |

## Quick Start

```bash
# 1. Build an agent image
cd claude-code
docker build -t agentkernel/claude-code .

# 2. Create a sandbox with the image
agentkernel create my-project --config agentkernel.toml --dir /path/to/project

# 3. Start and attach
agentkernel start my-project
agentkernel attach my-project

# 4. Run the agent inside the sandbox
claude  # or codex, gemini
```

## Building All Images

```bash
# Build all agent images
for agent in claude-code codex gemini; do
  echo "Building $agent..."
  docker build -t agentkernel/$agent $agent/
done
```

## Common Features

All agent images include:
- **Node.js 22** - Runtime for agent CLIs
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast search (used by agents)
- **fd** - Fast file finder
- **jq** - JSON processing
- **bash** - Shell

## Security

All images:
- Run as non-root user `developer`
- Have workspace isolated at `/workspace`
- Respect agentkernel security profiles

## Customizing

Create your own Dockerfile extending these base images:

```dockerfile
FROM agentkernel/claude-code

# Add your tools
RUN apk add --no-cache your-tools

# Add project-specific configuration
COPY .claude /home/developer/.claude/
```

## Environment Variables

Pass API keys when creating sandboxes. The keys are passed through to the container environment.

See individual agent READMEs for specific environment variable requirements.
