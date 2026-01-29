"""Sandbox session example for agentkernel Python SDK."""

from agentkernel import AgentKernel

client = AgentKernel()

# Create a sandbox session — auto-removed when context manager exits
with client.sandbox("demo", image="python:3.12-alpine") as sb:
    # Install a package
    sb.run(["pip", "install", "numpy"])

    # Run code
    result = sb.run(["python3", "-c", "import numpy; print(f'numpy {numpy.__version__}')"])
    print(result.output)

# Sandbox is automatically removed here
