
# AI Agents

agentkernel provides pre-configured Docker images for popular AI coding agents. These images include all necessary dependencies and are optimized for sandbox execution.

## Supported Agents

| Agent | CLI Command | API Key Variable |
|-------|-------------|------------------|
| [Claude Code](agent-claude) | `claude` | `ANTHROPIC_API_KEY` |
| [OpenAI Codex](agent-codex) | `codex` | `OPENAI_API_KEY` |
| [Google Gemini](agent-gemini) | `gemini` | `GEMINI_API_KEY` |

## Quick Start

```bash
# Create a sandbox with Claude Code
agentkernel create my-agent --config examples/agents/claude-code/agentkernel.toml

# Start and run with your API key
agentkernel start my-agent
agentkernel exec my-agent -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY -- claude -p "Hello"
```

## Common Features

All agent images include:

- **Node.js 22** - Runtime for agent CLIs
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast code search
- **fd** - Fast file finder
- **jq** - JSON processing
- **bash** - Shell

## Security

All agent images:

- Run as non-root user `developer`
- Have workspace isolated at `/workspace`
- Respect agentkernel security profiles
- Require explicit API key passthrough (not inherited from host)

## Custom Agent Images

Create your own agent image by extending the base:

```dockerfile
FROM agentkernel/claude-code

# Add your tools
RUN apk add --no-cache your-tools

# Add project-specific configuration
COPY .claude /home/developer/.claude/
```

Then reference it in your config:

```toml
[build]
dockerfile = "Dockerfile.agent"
```
