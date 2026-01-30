
# Claude Code

Use [Claude Code](https://claude.ai/code) with agentkernel for isolated code execution.

## Plugin Mode (Recommended)

Claude runs locally, code execution is sandboxed via MCP:

```bash
# Install the plugin into your project
agentkernel plugin install claude

# This creates:
#   .claude/skills/agentkernel/SKILL.md   — teaches Claude when/how to use sandboxes
#   .claude/commands/sandbox.md           — adds /sandbox command
#   .mcp.json                             — registers the agentkernel MCP server
```

Once installed, Claude can use sandboxed execution automatically via the skill, or you can invoke it explicitly:

```
/sandbox python3 -c "print('Hello from sandbox!')"
/sandbox npm test
/sandbox --profile restrictive python3 untrusted.py
```

For global installation (available in all projects):

```bash
agentkernel plugin install claude --global
```

## Sandbox Mode

Run Claude Code itself inside an isolated sandbox:

```bash
# Create sandbox with Claude Code pre-installed
agentkernel create claude-dev --config examples/agents/claude-code/agentkernel.toml

# Start the sandbox
agentkernel start claude-dev

# Run Claude with your API key
agentkernel attach claude-dev -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY

# Inside the sandbox:
claude
```

## API Key

Claude Code requires an Anthropic API key. Get one from [console.anthropic.com](https://console.anthropic.com/).

**Important:** Use a regular API key (`sk-ant-api03-...`), not an OAuth token.

Pass the key when running commands:

```bash
# Interactive session
agentkernel attach claude-dev -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY

# One-off command
agentkernel exec claude-dev -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY -- \
  claude -p "Explain this code"
```

## Configuration

The example config at `examples/agents/claude-code/agentkernel.toml`:

```toml
[sandbox]
name = "claude-code-sandbox"

[build]
dockerfile = "Dockerfile"

[agent]
preferred = "claude"
compatibility_mode = "claude"

[resources]
vcpus = 2
memory_mb = 1024

[security]
profile = "moderate"
network = true      # Claude needs network for API calls
mount_cwd = true    # Mount project directory
```

## What's Included

The Claude Code image includes:

- **Node.js 22** - Runtime for Claude Code
- **Claude Code CLI** - `@anthropic-ai/claude-code`
- **Git** - Version control
- **Python 3** - For Python projects
- **ripgrep** - Fast search (used by Claude Code)
- **fd** - Fast file finder
- **jq** - JSON processing

## Working with Projects

Mount your project into the sandbox:

```bash
# Create with your project
agentkernel create my-project \
  --config examples/agents/claude-code/agentkernel.toml \
  --dir /path/to/your/project

# Your code is at /workspace inside the sandbox
agentkernel attach my-project -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY
claude  # Claude can now access your project
```

## Permissions Mode

For fully autonomous operation, Claude Code supports `--dangerously-skip-permissions`:

```bash
agentkernel exec claude-dev -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY -- \
  claude --dangerously-skip-permissions -p "Fix all linting errors"
```

This is safe inside agentkernel because the sandbox provides isolation.

## Customizing

Create a custom Dockerfile based on the example:

```dockerfile
FROM node:22-alpine

# Base tools
RUN apk add --no-cache git bash python3 ripgrep fd jq

# Claude Code CLI
RUN npm install -g @anthropic-ai/claude-code

# Your additions
RUN apk add --no-cache rust cargo
COPY .claude /home/developer/.claude/

# Setup
WORKDIR /workspace
USER developer
```
