
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

When automatic backend selection chooses Firecracker, create delegates the
start and any requested repository setup to the local API server.

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

Removing a Firecracker sandbox is delegated to the local API server so the
server-owned VM is stopped before its state is deleted.

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

Firecracker start and stop operations are delegated to the local API server.
Set `AGENTKERNEL_API_KEY` when that server requires bearer authentication.

### sandbox_pause / sandbox_resume

Pause and resume a Firecracker sandbox with guest memory and process state
preserved. These operations require Firecracker on x86_64 Linux/KVM and a healthy
local `agentkernel serve` process. The MCP server delegates Firecracker
lifecycle operations to that long-running process so the VM survives after a
tool call returns. `AGENTKERNEL_PORT` selects a non-default local API port.

```json
{
  "name": "sandbox_pause",
  "arguments": {
    "name": "experiment-a"
  }
}
```

```json
{
  "name": "sandbox_resume",
  "arguments": {
    "name": "experiment-a"
  }
}
```

### sandbox_fork

Fork a paused Firecracker sandbox into a new running sandbox. The source stays
paused and reusable.

```json
{
  "name": "sandbox_fork",
  "arguments": {
    "name": "experiment-a",
    "as_name": "experiment-b"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Paused source sandbox |
| `as_name` | string | Yes | Name for the new running sandbox |

The response includes a security warning because the child receives a copy of
guest memory and filesystem state. Credentials captured in the checkpoint are
duplicated; rotate or revoke them when appropriate. Fork uses the existing
`sandbox_create` interactive permission for the child name. Enterprise policy
requires Run on the source plus both Create and Run for the child. The child
inherits the source owner and tenant metadata; cross-owner or cross-tenant forks
are rejected before restore.

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

### sandbox_extend_ttl

Extend a sandbox's time-to-live.

```json
{
  "name": "sandbox_extend_ttl",
  "arguments": {
    "name": "my-sandbox",
    "by": "1h"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Sandbox name |
| `by` | string | No | Duration to extend (default: "1h") |

### snapshot_list

List all snapshots.

```json
{
  "name": "snapshot_list",
  "arguments": {}
}
```

### snapshot_take

Take a snapshot of a sandbox.

```json
{
  "name": "snapshot_take",
  "arguments": {
    "sandbox": "my-sandbox",
    "name": "checkpoint-1"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `sandbox` | string | Yes | Sandbox to snapshot |
| `name` | string | Yes | Snapshot name |

### snapshot_get

Get information about a snapshot.

```json
{
  "name": "snapshot_get",
  "arguments": {
    "name": "checkpoint-1"
  }
}
```

### snapshot_delete

Delete a snapshot.

```json
{
  "name": "snapshot_delete",
  "arguments": {
    "name": "checkpoint-1"
  }
}
```

### snapshot_restore

Restore a sandbox from a snapshot.

```json
{
  "name": "snapshot_restore",
  "arguments": {
    "name": "checkpoint-1",
    "as_name": "restored-sandbox"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Snapshot to restore |
| `as_name` | string | No | Name for restored sandbox (default: "{original}-restored") |

### Browser Tools

These tools provide ARIA-based browser automation with persistent pages. The browser server is auto-started on first use.

#### browser_open

Navigate to a URL and return an ARIA accessibility tree snapshot.

```json
{
  "name": "browser_open",
  "arguments": {
    "name": "my-browser",
    "url": "https://example.com",
    "page": "default"
  }
}
```

Returns an ARIA snapshot with `snapshot` (YAML tree), `url`, `title`, and `refs` (interactive element IDs).

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Sandbox name |
| `url` | string | Yes | URL to navigate to |
| `page` | string | No | Page name (default: "default") |

#### browser_snapshot

Get the current ARIA snapshot without navigating.

```json
{
  "name": "browser_snapshot",
  "arguments": {
    "name": "my-browser",
    "page": "default"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Sandbox name |
| `page` | string | No | Page name (default: "default") |

#### browser_click

Click an element by ref ID or CSS selector. Returns a new ARIA snapshot.

```json
{
  "name": "browser_click",
  "arguments": {
    "name": "my-browser",
    "ref": "e2"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Sandbox name |
| `ref` | string | No | Ref ID from ARIA snapshot (e.g. "e2") |
| `selector` | string | No | CSS selector (fallback if no ref) |
| `page` | string | No | Page name (default: "default") |

#### browser_fill

Fill an input field by ref ID or CSS selector. Returns a new ARIA snapshot.

```json
{
  "name": "browser_fill",
  "arguments": {
    "name": "my-browser",
    "ref": "e3",
    "value": "search query"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Sandbox name |
| `value` | string | Yes | Text to type |
| `ref` | string | No | Ref ID from ARIA snapshot |
| `selector` | string | No | CSS selector (fallback if no ref) |
| `page` | string | No | Page name (default: "default") |

#### browser_close

Close a named browser page.

```json
{
  "name": "browser_close",
  "arguments": {
    "name": "my-browser",
    "page": "default"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Sandbox name |
| `page` | string | No | Page name to close (default: "default") |

#### browser_events

Retrieve sequenced browser interaction events. Useful for debugging and context recovery.

```json
{
  "name": "browser_events",
  "arguments": {
    "name": "my-browser",
    "offset": 0,
    "limit": 50
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Sandbox name |
| `offset` | integer | No | Start from this sequence number (default: 0) |
| `limit` | integer | No | Max events to return (default: 100) |

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
