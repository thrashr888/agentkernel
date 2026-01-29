"""Streaming example for agentkernel Python SDK."""

import sys

from agentkernel import AgentKernel

client = AgentKernel()

# Stream output from a command
for event in client.run_stream(["python3", "-c", "print('Hello from streaming!')"]):
    match event.event_type:
        case "started":
            print("[started]", event.data)
        case "output":
            sys.stdout.write(event.data.get("data", ""))
        case "done":
            print(f"\n[done] exit_code: {event.data.get('exit_code')}")
        case "error":
            print(f"[error] {event.data.get('message')}", file=sys.stderr)
