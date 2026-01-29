"""Async example for agentkernel Python SDK."""

import asyncio
import sys

from agentkernel import AsyncAgentKernel


async def main() -> None:
    async with AsyncAgentKernel() as client:
        # Health check
        print("Health:", await client.health())

        # Run a command
        result = await client.run(["echo", "Hello from async agentkernel!"])
        print("Output:", result.output)

        # Sandbox session with async context manager
        async with client.sandbox("async-demo", image="python:3.12-alpine") as sb:
            await sb.run(["pip", "install", "requests"])
            result = await sb.run(["python3", "-c", "import requests; print(requests.__version__)"])
            print(result.output)
        # sandbox auto-removed

        # Async streaming
        async for event in client.run_stream(["echo", "streaming async"]):
            if event.event_type == "output":
                sys.stdout.write(event.data.get("data", ""))


asyncio.run(main())
