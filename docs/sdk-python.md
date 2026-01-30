
# Python SDK

Official Python client for agentkernel. Supports both sync and async usage.

- **Package**: [`agentkernel`](https://pypi.org/project/agentkernel/)
- **Source**: [`sdk/python/`](https://github.com/thrashr888/agentkernel/tree/main/sdk/python)
- **Requires**: Python 3.10+

## Install

```bash
pip install agentkernel
```

## Quick Start

```python
from agentkernel import AgentKernel

with AgentKernel() as client:
    result = client.run(["echo", "hello"])
    print(result.output)  # "hello\n"
```

## Async Usage

```python
from agentkernel import AsyncAgentKernel

async with AsyncAgentKernel() as client:
    result = await client.run(["echo", "hello"])
    print(result.output)
```

## Configuration

```python
client = AgentKernel(
    base_url="http://localhost:18888",  # default
    api_key="sk-...",                  # optional
    timeout=30.0,                      # default: 30s
)
```

Or use environment variables:

```bash
export AGENTKERNEL_BASE_URL=http://localhost:18888
export AGENTKERNEL_API_KEY=sk-...
```

## Running Commands

### Basic Execution

```python
with AgentKernel() as client:
    result = client.run(["python3", "-c", "print(1 + 1)"])
    print(result.output)  # "2\n"
```

### With Options

```python
result = client.run(
    ["npm", "test"],
    image="node:22-alpine",
    profile="restrictive",
    fast=False,
)
```

### Streaming Output

Sync streaming:

```python
for event in client.run_stream(["python3", "script.py"]):
    if event.type == "output":
        print(event.data["data"], end="")
    elif event.type == "done":
        print(f"Exit code: {event.data['exit_code']}")
    elif event.type == "error":
        print(f"Error: {event.data['message']}")
```

Async streaming:

```python
async for event in client.run_stream(["python3", "script.py"]):
    if event.type == "output":
        print(event.data["data"], end="")
```

## Sandbox Management

### Create and Execute

```python
# Create a sandbox
sandbox = client.create_sandbox("my-project", image="python:3.12-alpine")

# Execute commands
result = client.exec_in_sandbox("my-project", ["pip", "install", "numpy"])

# Get info
info = client.get_sandbox("my-project")

# List all
sandboxes = client.list_sandboxes()

# Remove
client.remove_sandbox("my-project")
```

### Sandbox Sessions (Recommended)

The `sandbox()` context manager creates a sandbox and removes it when the block exits:

```python
with AgentKernel() as client:
    with client.sandbox("test", image="python:3.12-alpine") as sb:
        sb.run(["pip", "install", "numpy"])
        result = sb.run(["python3", "-c", "import numpy; print(numpy.__version__)"])
        print(result.output)
    # sandbox auto-removed
```

Async version:

```python
async with AsyncAgentKernel() as client:
    async with client.sandbox("test", image="python:3.12-alpine") as sb:
        await sb.run(["pip", "install", "numpy"])
        result = await sb.run(["python3", "-c", "import numpy; print(numpy.__version__)"])
        print(result.output)
```

## Error Handling

```python
from agentkernel import AgentKernel, AgentKernelError

with AgentKernel() as client:
    try:
        client.run(["bad-command"])
    except AgentKernelError as e:
        print(e.status)   # HTTP status code
        print(e.message)  # Error message from server
```

Error types by HTTP status:

| Status | Error | Description |
|--------|-------|-------------|
| 400 | `AgentKernelError` | Validation error |
| 401 | `AgentKernelError` | Authentication error |
| 404 | `AgentKernelError` | Resource not found |
| 500 | `AgentKernelError` | Server error |

## API Reference

### Sync Client (`AgentKernel`)

| Method | Returns | Description |
|--------|---------|-------------|
| `health()` | `str` | Health check |
| `run(command, **options)` | `RunOutput` | Run command in temporary sandbox |
| `run_stream(command, **options)` | `Iterator[StreamEvent]` | Run with streaming output |
| `list_sandboxes()` | `list[SandboxInfo]` | List all sandboxes |
| `create_sandbox(name, **options)` | `SandboxInfo` | Create a sandbox |
| `get_sandbox(name)` | `SandboxInfo` | Get sandbox info |
| `remove_sandbox(name)` | `None` | Remove a sandbox |
| `exec_in_sandbox(name, command)` | `RunOutput` | Execute in existing sandbox |
| `sandbox(name, **options)` | `SandboxSession` | Context manager with auto-cleanup |

### Async Client (`AsyncAgentKernel`)

Same methods as above, but all return coroutines (`await`-able). `run_stream` returns `AsyncIterator[StreamEvent]`.
