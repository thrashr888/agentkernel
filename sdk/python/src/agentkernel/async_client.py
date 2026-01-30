"""Asynchronous client for the agentkernel HTTP API."""

from __future__ import annotations

from collections.abc import AsyncIterator
from types import TracebackType
from typing import Any

import httpx

from ._config import resolve_config
from .errors import AgentKernelError, NetworkError, error_from_status
from .types import (
    BatchRunResponse,
    CreateSandboxOptions,
    FileReadResponse,
    RunOptions,
    RunOutput,
    SandboxInfo,
    SecurityProfile,
    StreamEvent,
)

SDK_VERSION = "0.4.0"


class AsyncSandboxSession:
    """An async sandbox session with auto-cleanup on context manager exit."""

    def __init__(self, name: str, client: AsyncAgentKernel) -> None:
        self.name = name
        self._client = client
        self._removed = False

    async def run(self, command: list[str]) -> RunOutput:
        """Run a command in this sandbox."""
        return await self._client.exec_in_sandbox(self.name, command)

    async def info(self) -> SandboxInfo:
        """Get sandbox info."""
        return await self._client.get_sandbox(self.name)

    async def remove(self) -> None:
        """Remove the sandbox. Idempotent."""
        if self._removed:
            return
        self._removed = True
        await self._client.remove_sandbox(self.name)

    async def __aenter__(self) -> AsyncSandboxSession:
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: TracebackType | None,
    ) -> None:
        await self.remove()


class AsyncAgentKernel:
    """Asynchronous client for the agentkernel HTTP API.

    Example::

        async with AsyncAgentKernel() as client:
            result = await client.run(["echo", "hello"])
            print(result.output)
    """

    def __init__(
        self,
        base_url: str | None = None,
        api_key: str | None = None,
        timeout: float | None = None,
    ) -> None:
        config = resolve_config(base_url, api_key, timeout)
        headers: dict[str, str] = {"User-Agent": f"agentkernel-python-sdk/{SDK_VERSION}"}
        if config.api_key:
            headers["Authorization"] = f"Bearer {config.api_key}"
        self._http = httpx.AsyncClient(
            base_url=config.base_url,
            headers=headers,
            timeout=config.timeout,
        )

    async def close(self) -> None:
        """Close the HTTP client."""
        await self._http.aclose()

    async def __aenter__(self) -> AsyncAgentKernel:
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: TracebackType | None,
    ) -> None:
        await self.close()

    # -- API methods --

    async def health(self) -> str:
        """Health check. Returns 'ok'."""
        return await self._request("GET", "/health")

    async def run(
        self,
        command: list[str],
        *,
        image: str | None = None,
        profile: SecurityProfile | None = None,
        fast: bool = True,
    ) -> RunOutput:
        """Run a command in a temporary sandbox."""
        data = await self._request(
            "POST",
            "/run",
            json={"command": command, "image": image, "profile": profile, "fast": fast},
        )
        return RunOutput(**data)

    async def run_stream(
        self,
        command: list[str],
        *,
        image: str | None = None,
        profile: SecurityProfile | None = None,
        fast: bool = True,
    ) -> AsyncIterator[StreamEvent]:
        """Run a command with SSE streaming output."""
        from .sse import iter_sse_async

        response = await self._http.send(
            self._http.build_request(
                "POST",
                "/run/stream",
                json={"command": command, "image": image, "profile": profile, "fast": fast},
            ),
            stream=True,
        )
        if response.status_code >= 400:
            await response.aread()
            raise error_from_status(response.status_code, response.text)
        return iter_sse_async(response)

    async def list_sandboxes(self) -> list[SandboxInfo]:
        """List all sandboxes."""
        data = await self._request("GET", "/sandboxes")
        return [SandboxInfo(**s) for s in data]

    async def create_sandbox(
        self,
        name: str,
        *,
        image: str | None = None,
        vcpus: int | None = None,
        memory_mb: int | None = None,
        profile: SecurityProfile | None = None,
    ) -> SandboxInfo:
        """Create a new sandbox."""
        data = await self._request(
            "POST",
            "/sandboxes",
            json={"name": name, "image": image, "vcpus": vcpus, "memory_mb": memory_mb, "profile": profile},
        )
        return SandboxInfo(**data)

    async def get_sandbox(self, name: str) -> SandboxInfo:
        """Get info about a sandbox."""
        data = await self._request("GET", f"/sandboxes/{name}")
        return SandboxInfo(**data)

    async def remove_sandbox(self, name: str) -> None:
        """Remove a sandbox."""
        await self._request("DELETE", f"/sandboxes/{name}")

    async def exec_in_sandbox(self, name: str, command: list[str]) -> RunOutput:
        """Run a command in an existing sandbox."""
        data = await self._request("POST", f"/sandboxes/{name}/exec", json={"command": command})
        return RunOutput(**data)

    async def read_file(self, name: str, path: str) -> FileReadResponse:
        """Read a file from a sandbox."""
        data = await self._request("GET", f"/sandboxes/{name}/files/{path}")
        return FileReadResponse(**data)

    async def write_file(
        self,
        name: str,
        path: str,
        content: str,
        *,
        encoding: str = "utf8",
    ) -> str:
        """Write a file to a sandbox."""
        return await self._request(
            "PUT",
            f"/sandboxes/{name}/files/{path}",
            json={"content": content, "encoding": encoding},
        )

    async def delete_file(self, name: str, path: str) -> str:
        """Delete a file from a sandbox."""
        return await self._request("DELETE", f"/sandboxes/{name}/files/{path}")

    async def get_sandbox_logs(self, name: str) -> list[dict]:
        """Get audit log entries for a sandbox."""
        return await self._request("GET", f"/sandboxes/{name}/logs")

    async def batch_run(self, commands: list[list[str]]) -> BatchRunResponse:
        """Run multiple commands in parallel."""
        batch_commands = [{"command": cmd} for cmd in commands]
        data = await self._request("POST", "/batch/run", json={"commands": batch_commands})
        return BatchRunResponse(**data)

    async def sandbox(
        self,
        name: str,
        *,
        image: str | None = None,
        vcpus: int | None = None,
        memory_mb: int | None = None,
        profile: SecurityProfile | None = None,
    ) -> AsyncSandboxSession:
        """Create a sandbox session with automatic cleanup.

        Example::

            async with await client.sandbox("test") as sb:
                await sb.run(["echo", "hello"])
            # sandbox auto-removed
        """
        await self.create_sandbox(name, image=image, vcpus=vcpus, memory_mb=memory_mb, profile=profile)
        return AsyncSandboxSession(name, self)

    # -- Internal --

    async def _request(self, method: str, path: str, **kwargs: Any) -> Any:
        try:
            response = await self._http.request(method, path, **kwargs)
        except httpx.ConnectError as e:
            raise NetworkError(f"Failed to connect: {e}") from e
        except httpx.TimeoutException as e:
            raise NetworkError(f"Request timed out: {e}") from e

        if response.status_code >= 400:
            raise error_from_status(response.status_code, response.text)

        data = response.json()
        if not data.get("success"):
            raise AgentKernelError(data.get("error", "Unknown error"))
        return data.get("data")
