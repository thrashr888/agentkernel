# agentkernel MCP Server

Use agentkernel with any MCP-compatible AI coding agent.

agentkernel includes a built-in MCP (Model Context Protocol) server that lets any compatible agent run commands in hardware-isolated microVM sandboxes.

## Compatible Agents

| Agent | Config Location | Status |
|-------|----------------|--------|
| Claude Code | `~/.claude/settings.json` | Built-in (see `claude-plugin/`) |
| Codex | MCP config file | See `plugins/codex/` |
| Gemini CLI | `~/.gemini/settings.json` | See `plugins/gemini/` |
| OpenCode | `.opencode/plugins/` | See `plugins/opencode/` |
| Any MCP agent | Varies | Use config below |

## Universal MCP Config

For any MCP-compatible agent, add this server configuration:

```json
{
  "mcpServers": {
    "agentkernel": {
      "command": "agentkernel",
      "args": ["mcp-server"],
      "env": {}
    }
  }
}
```

With an API key:

```json
{
  "mcpServers": {
    "agentkernel": {
      "command": "agentkernel",
      "args": ["mcp-server"],
      "env": {
        "AGENTKERNEL_API_KEY": "sk-..."
      }
    }
  }
}
```

## MCP Tools

The server exposes these tools:

| Tool | Description |
|------|-------------|
| `run_command` | Run a command in a temporary sandbox |
| `create_sandbox` | Create a named persistent sandbox |
| `exec_in_sandbox` | Execute a command in an existing sandbox |
| `remove_sandbox` | Remove a sandbox |
| `list_sandboxes` | List all active sandboxes |

## Prerequisites

Install agentkernel:

```bash
# Homebrew
brew tap thrashr888/agentkernel && brew install agentkernel

# Cargo
cargo install --git https://github.com/thrashr888/agentkernel

# From source
git clone https://github.com/thrashr888/agentkernel
cd agentkernel && cargo build --release
```

## License

MIT
