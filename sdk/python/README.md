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
