# agentkernel for Gemini CLI

Use agentkernel as a sandbox backend for Google Gemini CLI via MCP.

## Setup

1. Install agentkernel:

```bash
brew tap thrashr888/agentkernel && brew install agentkernel
```

2. Add the MCP server to your Gemini CLI config. Add to your `~/.gemini/settings.json` or project-level `.gemini/settings.json`:

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

3. Start using agentkernel tools in Gemini CLI. The MCP server exposes:

| Tool | Description |
|------|-------------|
| `run_command` | Run a command in a temporary sandbox |
| `create_sandbox` | Create a persistent sandbox |
| `exec_in_sandbox` | Execute in an existing sandbox |
| `remove_sandbox` | Remove a sandbox |
| `list_sandboxes` | List all sandboxes |

## How It Works

The `agentkernel mcp-server` command starts a stdio-based MCP server that Gemini CLI connects to. All code execution is routed through agentkernel's microVM sandboxes.

## License

MIT
