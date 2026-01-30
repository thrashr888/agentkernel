"""Synchronous client for the agentkernel HTTP API."""

from __future__ import annotations

from collections.abc import Iterator
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


class SandboxSession:
    """A sandbox session with auto-cleanup on context manager exit."""

    def __init__(self, name: str, client: AgentKernel) -> None:
        self.name = name
        self._client = client
        self._removed = False

    def run(self, command: list[str]) -> RunOutput:
        """Run a command in this sandbox."""
        return self._client.exec_in_sandbox(self.name, command)

    def info(self) -> SandboxInfo:
        """Get sandbox info."""
        return self._client.get_sandbox(self.name)

    def remove(self) -> None:
        """Remove the sandbox. Idempotent."""
        if self._removed:
            return
        self._removed = True
        self._client.remove_sandbox(self.name)

    def __enter__(self) -> SandboxSession:
        return self

    def __exit__(self, *args: Any) -> None:
        self.remove()


class AgentKernel:
    """Synchronous client for the agentkernel HTTP API.

    Example::

        with AgentKernel() as client:
            result = client.run(["echo", "hello"])
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
        self._http = httpx.Client(
            base_url=config.base_url,
            headers=headers,
            timeout=config.timeout,
        )

    def close(self) -> None:
        """Close the HTTP client."""
        self._http.close()

    def __enter__(self) -> AgentKernel:
        return self

    def __exit__(self, *args: Any) -> None:
        self.close()

    # -- API methods --

    def health(self) -> str:
        """Health check. Returns 'ok'."""
        return self._request("GET", "/health")

    def run(
        self,
        command: list[str],
        *,
        image: str | None = None,
        profile: SecurityProfile | None = None,
        fast: bool = True,
    ) -> RunOutput:
        """Run a command in a temporary sandbox."""
        data = self._request(
            "POST",
            "/run",
            json={"command": command, "image": image, "profile": profile, "fast": fast},
        )
        return RunOutput(**data)

    def run_stream(
        self,
        command: list[str],
        *,
        image: str | None = None,
        profile: SecurityProfile | None = None,
        fast: bool = True,
    ) -> Iterator[StreamEvent]:
        """Run a command with SSE streaming output."""
        from .sse import iter_sse_sync

        with self._http.stream(
            "POST",
            "/run/stream",
            json={"command": command, "image": image, "profile": profile, "fast": fast},
        ) as response:
            if response.status_code >= 400:
                response.read()
                raise error_from_status(response.status_code, response.text)
            yield from iter_sse_sync(response)

    def list_sandboxes(self) -> list[SandboxInfo]:
        """List all sandboxes."""
        data = self._request("GET", "/sandboxes")
        return [SandboxInfo(**s) for s in data]

    def create_sandbox(
        self,
        name: str,
        *,
        image: str | None = None,
        vcpus: int | None = None,
        memory_mb: int | None = None,
        profile: SecurityProfile | None = None,
    ) -> SandboxInfo:
        """Create a new sandbox."""
        data = self._request(
            "POST",
            "/sandboxes",
            json={"name": name, "image": image, "vcpus": vcpus, "memory_mb": memory_mb, "profile": profile},
        )
        return SandboxInfo(**data)

    def get_sandbox(self, name: str) -> SandboxInfo:
        """Get info about a sandbox."""
        data = self._request("GET", f"/sandboxes/{name}")
        return SandboxInfo(**data)

    def remove_sandbox(self, name: str) -> None:
        """Remove a sandbox."""
        self._request("DELETE", f"/sandboxes/{name}")

    def exec_in_sandbox(self, name: str, command: list[str]) -> RunOutput:
        """Run a command in an existing sandbox."""
        data = self._request("POST", f"/sandboxes/{name}/exec", json={"command": command})
        return RunOutput(**data)

    def read_file(self, name: str, path: str) -> FileReadResponse:
        """Read a file from a sandbox."""
        data = self._request("GET", f"/sandboxes/{name}/files/{path}")
        return FileReadResponse(**data)

    def write_file(
        self,
        name: str,
        path: str,
        content: str,
        *,
        encoding: str = "utf8",
    ) -> str:
        """Write a file to a sandbox."""
        return self._request(
            "PUT",
            f"/sandboxes/{name}/files/{path}",
            json={"content": content, "encoding": encoding},
        )

    def delete_file(self, name: str, path: str) -> str:
        """Delete a file from a sandbox."""
        return self._request("DELETE", f"/sandboxes/{name}/files/{path}")

    def get_sandbox_logs(self, name: str) -> list[dict]:
        """Get audit log entries for a sandbox."""
        return self._request("GET", f"/sandboxes/{name}/logs")

    def batch_run(self, commands: list[list[str]]) -> BatchRunResponse:
        """Run multiple commands in parallel."""
        batch_commands = [{"command": cmd} for cmd in commands]
        data = self._request("POST", "/batch/run", json={"commands": batch_commands})
        return BatchRunResponse(**data)

    def sandbox(
        self,
        name: str,
        *,
        image: str | None = None,
        vcpus: int | None = None,
        memory_mb: int | None = None,
        profile: SecurityProfile | None = None,
    ) -> SandboxSession:
        """Create a sandbox session with automatic cleanup.

        Example::

            with client.sandbox("test", image="python:3.12-alpine") as sb:
                sb.run(["pip", "install", "numpy"])
            # sandbox auto-removed
        """
        self.create_sandbox(name, image=image, vcpus=vcpus, memory_mb=memory_mb, profile=profile)
        return SandboxSession(name, self)

    # -- Internal --

    def _request(self, method: str, path: str, **kwargs: Any) -> Any:
        try:
            response = self._http.request(method, path, **kwargs)
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
