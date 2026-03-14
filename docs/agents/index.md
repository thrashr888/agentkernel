
# AI Agents

agentkernel provides pre-configured Docker images for popular AI coding agents. These images include all necessary dependencies and are optimized for sandbox execution.

## Supported Agents

| Agent | CLI Command | API Key Variable |
|-------|-------------|------------------|
| [Claude Code](claude.md) | `claude` | `ANTHROPIC_API_KEY` |
| [OpenAI Codex](codex.md) | `codex` | `OPENAI_API_KEY` |
| [Google Gemini](gemini.md) | `gemini` | `GEMINI_API_KEY` |
| [GitHub Copilot](copilot.md) | `github-copilot` | `GITHUB_TOKEN` |
| [Amp](amp.md) | `amp` | `ANTHROPIC_API_KEY` |
| [Pi](pi.md) | `pi` | `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` |
| [OpenCode](opencode.md) | `opencode` | Provider-specific |
| [Hermes Agent](hermes.md) | `hermes` | `OPENROUTER_API_KEY` or `ANTHROPIC_API_KEY` |
| [Symphony](symphony.md) | `symphony` | `LINEAR_API_KEY` + `OPENAI_API_KEY` |

## Quick Start

### Plugin Mode (agent runs locally, code runs in sandbox)

```bash
# Install the plugin for your agent
agentkernel plugin install claude     # or: codex, gemini, copilot, amp, pi, opencode, mcp
```

### Sandbox Mode (agent runs inside the sandbox)

```bash
# Create a sandbox with Claude Code
agentkernel sandbox create my-agent --config examples/agents/claude-code/agentkernel.toml

# Start and run with your API key
agentkernel sandbox start my-agent
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
