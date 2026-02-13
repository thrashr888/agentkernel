# Agent Images for Agentkernel

Docker images for running AI coding agents in isolated sandboxes.
Images are built automatically from Dockerfiles when you create a sandbox.

## Available Agents

| Agent | Directory | API Key Env |
|-------|-----------|-------------|
| Claude Code | `claude-code/` | `ANTHROPIC_API_KEY` |
| OpenAI Codex | `codex/` | `OPENAI_API_KEY` |
| Google Gemini | `gemini/` | `GEMINI_API_KEY` |
| GitHub Copilot | `copilot/` | `GITHUB_TOKEN` |
| Amp | `amp/` | `ANTHROPIC_API_KEY` |
| Pi | `pi/` | `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` |
| OpenCode | `opencode/` | Provider-specific |

## Quick Start

```bash
# 1. Create a sandbox (builds image automatically from Dockerfile)
agentkernel sandbox create my-project --config examples/agents/claude-code/agentkernel.toml --backend docker

# 2. Start and attach
agentkernel sandbox start my-project --backend docker
agentkernel attach my-project

# 3. Run the agent inside the sandbox
claude  # or codex, gemini, amp, pi, copilot, opencode
```

No separate `docker build` step needed - agentkernel automatically builds from the Dockerfile specified in `agentkernel.toml`. Images are cached based on content hash, so subsequent creates reuse the cached image.

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
