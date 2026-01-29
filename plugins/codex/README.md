# agentkernel for Codex

Use agentkernel as a sandbox backend for OpenAI Codex via MCP.

## Setup

1. Install agentkernel:

```bash
brew tap thrashr888/agentkernel && brew install agentkernel
```

2. Add the MCP server config to your Codex setup. Copy `mcp.json` to your Codex MCP configuration directory:

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

3. Start using agentkernel tools in Codex. The MCP server exposes:

| Tool | Description |
|------|-------------|
| `run_command` | Run a command in a temporary sandbox |
| `create_sandbox` | Create a persistent sandbox |
| `exec_in_sandbox` | Execute in an existing sandbox |
| `remove_sandbox` | Remove a sandbox |
| `list_sandboxes` | List all sandboxes |

## How It Works

The `agentkernel mcp-server` command starts a stdio-based MCP server that Codex connects to. All code execution is routed through agentkernel's microVM sandboxes.

## License

MIT
