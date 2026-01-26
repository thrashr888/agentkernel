
# MCP Server

agentkernel implements the [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) for integration with AI assistants like Claude Desktop.

## Starting the Server

```bash
agentkernel mcp-server
```

The server communicates via JSON-RPC over stdio (stdin/stdout).

## Claude Desktop Integration

Add to your Claude Desktop configuration (`~/.config/claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "agentkernel": {
      "command": "agentkernel",
      "args": ["mcp-server"]
    }
  }
}
```

Restart Claude Desktop. You can now ask Claude to run code in sandboxes.

## Available Tools

The MCP server exposes these tools to AI assistants:

### sandbox_run

Run a command in a temporary sandbox.

```json
{
  "name": "sandbox_run",
  "arguments": {
    "command": ["python3", "-c", "print('hello')"],
    "image": "python:3.12-alpine"
  }
}
```

### sandbox_create

Create a persistent sandbox.

```json
{
  "name": "sandbox_create",
  "arguments": {
    "name": "my-sandbox",
    "image": "node:22-alpine"
  }
}
```

### sandbox_exec

Execute a command in a running sandbox.

```json
{
  "name": "sandbox_exec",
  "arguments": {
    "name": "my-sandbox",
    "command": ["npm", "test"]
  }
}
```

### sandbox_list

List all sandboxes.

```json
{
  "name": "sandbox_list",
  "arguments": {}
}
```

### sandbox_remove

Remove a sandbox.

```json
{
  "name": "sandbox_remove",
  "arguments": {
    "name": "my-sandbox"
  }
}
```

### sandbox_start / sandbox_stop

Start or stop a sandbox.

```json
{
  "name": "sandbox_start",
  "arguments": {
    "name": "my-sandbox"
  }
}
```

### sandbox_file_write

Write a file to a sandbox.

```json
{
  "name": "sandbox_file_write",
  "arguments": {
    "name": "my-sandbox",
    "path": "/app/script.py",
    "content": "print('hello')"
  }
}
```

### sandbox_file_read

Read a file from a sandbox.

```json
{
  "name": "sandbox_file_read",
  "arguments": {
    "name": "my-sandbox",
    "path": "/app/script.py"
  }
}
```

## Example Conversation

With MCP configured, you can have conversations like:

> **You:** Run this Python code in a sandbox: `print(sum(range(100)))`
>
> **Claude:** I'll run that in an isolated sandbox.
> *[Uses sandbox_run tool]*
> The result is `4950`.

> **You:** Create a sandbox called "my-project" and install numpy
>
> **Claude:** I'll create the sandbox and install numpy.
> *[Uses sandbox_create, then sandbox_exec with pip install numpy]*
> Done! The sandbox "my-project" is ready with numpy installed.

## Protocol Details

The MCP server implements:

- JSON-RPC 2.0 over stdio
- MCP protocol version 2024-11-05
- Tool calling with structured arguments
- Error responses for invalid operations
