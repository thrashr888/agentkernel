# agentkernel

Python SDK for [agentkernel](https://github.com/thrashr888/agentkernel) — run AI coding agents in secure, isolated microVMs.

## Install

```bash
pip install agentkernel-sdk
```

Requires Python 3.10+.

## Quick Start

```python
from agentkernel import AgentKernel

with AgentKernel() as client:
    result = client.run(["echo", "hello"])
    print(result.output)  # "hello\n"
```

## Async

```python
from agentkernel import AsyncAgentKernel

async with AsyncAgentKernel() as client:
    result = await client.run(["echo", "hello"])
    print(result.output)
```

## Sandbox Sessions

```python
with AgentKernel() as client:
    with client.sandbox("test", image="python:3.12-alpine") as sb:
        sb.run(["pip", "install", "numpy"])
        result = sb.run(["python3", "-c", "import numpy; print(numpy.__version__)"])
        print(result.output)
    # sandbox auto-removed
```

## Exec Options

Run commands with a working directory, environment variables, or as root:

```python
with AgentKernel() as client:
    result = client.exec_in_sandbox(
        "my-sandbox",
        ["npm", "start"],
        workdir="/app",
        env=["NODE_ENV=production"],
        sudo=True,
    )
```

Works on sandbox sessions too:

```python
with client.sandbox("dev") as sb:
    sb.run(["pip", "install", "-r", "requirements.txt"], workdir="/app", sudo=True)
```

## Git Source Cloning

Clone a git repo into the sandbox at creation time:

```python
with AgentKernel() as client:
    client.create_sandbox(
        "my-project",
        image="node:20-alpine",
        source_url="https://github.com/user/repo.git",
        source_ref="main",
    )
```

## File Operations

Read, write, and delete files in a sandbox:

```python
with AgentKernel() as client:
    # Write a file
    client.write_file("my-sandbox", "app/main.py", "print('hello')")

    # Read a file
    file = client.read_file("my-sandbox", "app/main.py")
    print(file.content)

    # Delete a file
    client.delete_file("my-sandbox", "app/main.py")

    # Batch write multiple files at once
    client.write_files("my-sandbox", {
        "/app/index.js": "console.log('hi')",
        "/app/package.json": '{"name":"app"}',
    })
```

## Streaming

```python
for event in client.run_stream(["python3", "script.py"]):
    if event.type == "output":
        print(event.data["data"], end="")
```

## Configuration

```python
client = AgentKernel(
    base_url="http://localhost:18888",  # default
    api_key="sk-...",                  # optional
    timeout=30.0,                      # default
)
```

Or use environment variables:

```bash
export AGENTKERNEL_BASE_URL=http://localhost:18888
export AGENTKERNEL_API_KEY=sk-...
```

## License

MIT
