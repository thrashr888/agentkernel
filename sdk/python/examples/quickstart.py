"""Quick start example for agentkernel Python SDK."""

from agentkernel import AgentKernel

client = AgentKernel()

# Health check
print("Health:", client.health())

# Run a command
result = client.run(["echo", "Hello from agentkernel!"])
print("Output:", result.output)

# List sandboxes
sandboxes = client.list_sandboxes()
print("Sandboxes:", sandboxes)
