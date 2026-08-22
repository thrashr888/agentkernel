"""Asynchronous client for the agentkernel HTTP API."""

from __future__ import annotations

from collections.abc import AsyncIterator
from types import TracebackType
from typing import TYPE_CHECKING, Any, cast

import httpx

if TYPE_CHECKING:
    from .browser import AsyncBrowserSession

from ._config import resolve_config
from .errors import AgentKernelError, NetworkError, error_from_status
from .types import (
    BatchFileWriteResponse,
    BatchRunResponse,
    DetachedCommand,
    DetachedLogsResponse,
    DurableObject,
    DurableStore,
    DurableStoreCommandResult,
    DurableStoreExecuteResult,
    DurableStoreQueryResult,
    FileReadResponse,
    Orchestration,
    OrchestrationDefinition,
    RunOutput,
    SandboxInfo,
    Schedule,
    SecurityProfile,
    SnapshotMeta,
    StreamEvent,
)

SDK_VERSION = "0.3.0"


class AsyncSandboxSession:
    """An async sandbox session with auto-cleanup on context manager exit."""

    def __init__(self, name: str, client: AsyncAgentKernel) -> None:
        self.name = name
        self._client = client
        self._removed = False

    async def run(
        self,
        command: list[str],
        *,
        env: list[str] | None = None,
        workdir: str | None = None,
        sudo: bool | None = None,
    ) -> RunOutput:
        """Run a command in this sandbox."""
        return await self._client.exec_in_sandbox(
            self.name,
            command,
            env=env,
            workdir=workdir,
            sudo=sudo,
        )

    async def info(self) -> SandboxInfo:
        """Get sandbox info."""
        return await self._client.get_sandbox(self.name)

    async def write_files(self, files: dict[str, str]) -> BatchFileWriteResponse:
        """Write multiple files in one request."""
        return await self._client.write_files(self.name, files)

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
        return cast(str, await self._request("GET", "/health"))

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
        source_url: str | None = None,
        source_ref: str | None = None,
        volumes: list[str] | None = None,
    ) -> SandboxInfo:
        """Create a new sandbox."""
        data = await self._request(
            "POST",
            "/sandboxes",
            json={
                "name": name,
                "image": image,
                "vcpus": vcpus,
                "memory_mb": memory_mb,
                "profile": profile,
                "source_url": source_url,
                "source_ref": source_ref,
                "volumes": volumes,
            },
        )
        return SandboxInfo(**data)

    async def get_sandbox(self, name: str) -> SandboxInfo:
        """Get info about a sandbox."""
        data = await self._request("GET", f"/sandboxes/{name}")
        return SandboxInfo(**data)

    async def get_sandbox_by_uuid(self, uuid: str) -> SandboxInfo:
        """Get info about a sandbox by UUID."""
        data = await self._request("GET", f"/sandboxes/by-uuid/{uuid}")
        return SandboxInfo(**data)

    async def remove_sandbox(self, name: str) -> None:
        """Remove a sandbox."""
        await self._request("DELETE", f"/sandboxes/{name}")

    async def exec_in_sandbox(
        self,
        name: str,
        command: list[str],
        *,
        env: list[str] | None = None,
        workdir: str | None = None,
        sudo: bool | None = None,
    ) -> RunOutput:
        """Run a command in an existing sandbox."""
        body: dict[str, Any] = {"command": command}
        if env:
            body["env"] = env
        if workdir is not None:
            body["workdir"] = workdir
        if sudo is not None:
            body["sudo"] = sudo
        data = await self._request("POST", f"/sandboxes/{name}/exec", json=body)
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
        return cast(
            str,
            await self._request(
                "PUT",
                f"/sandboxes/{name}/files/{path}",
                json={"content": content, "encoding": encoding},
            ),
        )

    async def delete_file(self, name: str, path: str) -> str:
        """Delete a file from a sandbox."""
        return cast(str, await self._request("DELETE", f"/sandboxes/{name}/files/{path}"))

    async def write_files(self, name: str, files: dict[str, str]) -> BatchFileWriteResponse:
        """Write multiple files to a sandbox in one request."""
        data = await self._request("POST", f"/sandboxes/{name}/files", json={"files": files})
        return BatchFileWriteResponse(**data)

    async def get_sandbox_logs(self, name: str) -> list[dict[str, Any]]:
        """Get audit log entries for a sandbox."""
        return cast(list[dict[str, Any]], await self._request("GET", f"/sandboxes/{name}/logs"))

    async def batch_run(self, commands: list[list[str]]) -> BatchRunResponse:
        """Run multiple commands in parallel."""
        batch_commands = [{"command": cmd} for cmd in commands]
        data = await self._request("POST", "/batch/run", json={"commands": batch_commands})
        return BatchRunResponse(**data)

    async def exec_detached(
        self,
        name: str,
        command: list[str],
        *,
        env: list[str] | None = None,
        workdir: str | None = None,
        sudo: bool | None = None,
    ) -> DetachedCommand:
        """Start a detached (background) command in a sandbox."""
        body: dict[str, Any] = {"command": command}
        if env:
            body["env"] = env
        if workdir is not None:
            body["workdir"] = workdir
        if sudo is not None:
            body["sudo"] = sudo
        data = await self._request("POST", f"/sandboxes/{name}/exec/detach", json=body)
        return DetachedCommand(**data)

    async def detached_status(self, name: str, cmd_id: str) -> DetachedCommand:
        """Get the status of a detached command."""
        data = await self._request("GET", f"/sandboxes/{name}/exec/detached/{cmd_id}")
        return DetachedCommand(**data)

    async def detached_logs(
        self, name: str, cmd_id: str, *, stream: str | None = None
    ) -> DetachedLogsResponse:
        """Get logs from a detached command."""
        query = f"?stream={stream}" if stream == "stderr" else ""
        data = await self._request("GET", f"/sandboxes/{name}/exec/detached/{cmd_id}/logs{query}")
        return DetachedLogsResponse(**data)

    async def detached_kill(self, name: str, cmd_id: str) -> str:
        """Kill a detached command."""
        return cast(
            str,
            await self._request("DELETE", f"/sandboxes/{name}/exec/detached/{cmd_id}"),
        )

    async def detached_list(self, name: str) -> list[DetachedCommand]:
        """List detached commands in a sandbox."""
        data = await self._request("GET", f"/sandboxes/{name}/exec/detached")
        return [DetachedCommand(**d) for d in data]

    async def list_orchestrations(self) -> list[Orchestration]:
        """List orchestrations."""
        return cast(list[Orchestration], await self._request("GET", "/orchestrations"))

    async def create_orchestration(self, orchestration: Orchestration) -> Orchestration:
        """Create a new orchestration."""
        return cast(
            Orchestration, await self._request("POST", "/orchestrations", json=orchestration)
        )

    async def get_orchestration(self, orchestration_id: str) -> Orchestration:
        """Get an orchestration by identifier."""
        return cast(
            Orchestration, await self._request("GET", f"/orchestrations/{orchestration_id}")
        )

    async def signal_orchestration(
        self,
        orchestration_id: str,
        event: dict[str, Any],
    ) -> Orchestration:
        """Raise an external event for an orchestration."""
        return cast(
            Orchestration,
            await self._request(
                "POST",
                f"/orchestrations/{orchestration_id}/events",
                json=event,
            ),
        )

    async def terminate_orchestration(
        self,
        orchestration_id: str,
        payload: dict[str, Any] | None = None,
    ) -> Orchestration:
        """Terminate an orchestration."""
        return cast(
            Orchestration,
            await self._request(
                "POST",
                f"/orchestrations/{orchestration_id}/terminate",
                json=payload or {},
            ),
        )

    async def list_orchestration_definitions(self) -> list[OrchestrationDefinition]:
        """List orchestration definitions."""
        return cast(
            list[OrchestrationDefinition],
            await self._request("GET", "/orchestrations/definitions"),
        )

    async def upsert_orchestration_definition(
        self,
        definition: OrchestrationDefinition,
    ) -> OrchestrationDefinition:
        """Register or update an orchestration definition."""
        return cast(
            OrchestrationDefinition,
            await self._request("POST", "/orchestrations/definitions", json=definition),
        )

    async def get_orchestration_definition(self, name: str) -> OrchestrationDefinition:
        """Get an orchestration definition by name."""
        return cast(
            OrchestrationDefinition,
            await self._request("GET", f"/orchestrations/definitions/{name}"),
        )

    async def delete_orchestration_definition(self, name: str) -> str:
        """Delete an orchestration definition by name."""
        return cast(str, await self._request("DELETE", f"/orchestrations/definitions/{name}"))

    async def list_objects(self) -> list[DurableObject]:
        """List objects."""
        return cast(list[DurableObject], await self._request("GET", "/objects"))

    async def create_object(self, obj: DurableObject) -> DurableObject:
        """Create a new object."""
        return cast(DurableObject, await self._request("POST", "/objects", json=obj))

    async def get_object(self, object_id: str) -> DurableObject:
        """Get an object by identifier."""
        return cast(DurableObject, await self._request("GET", f"/objects/{object_id}"))

    async def list_schedules(self) -> list[Schedule]:
        """List schedules."""
        return cast(list[Schedule], await self._request("GET", "/schedules"))

    async def create_schedule(self, schedule: Schedule) -> Schedule:
        """Create a new schedule."""
        return cast(Schedule, await self._request("POST", "/schedules", json=schedule))

    async def get_schedule(self, schedule_id: str) -> Schedule:
        """Get a schedule by identifier."""
        return cast(Schedule, await self._request("GET", f"/schedules/{schedule_id}"))

    async def list_stores(self) -> list[DurableStore]:
        """List durable stores."""
        return cast(list[DurableStore], await self._request("GET", "/stores"))

    async def create_store(self, store: DurableStore) -> DurableStore:
        """Create a durable store."""
        return cast(DurableStore, await self._request("POST", "/stores", json=store))

    async def get_store(self, store_id: str) -> DurableStore:
        """Get a durable store by identifier."""
        return cast(DurableStore, await self._request("GET", f"/stores/{store_id}"))

    async def delete_store(self, store_id: str) -> str:
        """Delete a durable store by identifier."""
        return cast(str, await self._request("DELETE", f"/stores/{store_id}"))

    async def query_store(
        self,
        store_id: str,
        payload: dict[str, Any],
    ) -> DurableStoreQueryResult:
        """Run a read query against a durable store."""
        return cast(
            DurableStoreQueryResult,
            await self._request("POST", f"/stores/{store_id}/query", json=payload),
        )

    async def execute_store(
        self,
        store_id: str,
        payload: dict[str, Any],
    ) -> DurableStoreExecuteResult:
        """Run a write statement against a durable store."""
        return cast(
            DurableStoreExecuteResult,
            await self._request("POST", f"/stores/{store_id}/execute", json=payload),
        )

    async def command_store(
        self,
        store_id: str,
        payload: dict[str, Any],
    ) -> DurableStoreCommandResult:
        """Run a command against a durable store (Redis-style engines)."""
        return cast(
            DurableStoreCommandResult,
            await self._request("POST", f"/stores/{store_id}/command", json=payload),
        )

    async def extend_ttl(self, name: str, *, by: str) -> str | None:
        """Extend a sandbox's TTL. Returns the new expiry time."""
        data = await self._request("POST", f"/sandboxes/{name}/extend", json={"by": by})
        return data.get("expires_at") if isinstance(data, dict) else data

    async def list_snapshots(self) -> list[SnapshotMeta]:
        """List all snapshots."""
        data = await self._request("GET", "/snapshots")
        return [SnapshotMeta(**s) for s in data]

    async def take_snapshot(
        self, sandbox: str, *, snapshot_name: str | None = None
    ) -> SnapshotMeta:
        """Take a snapshot of a sandbox."""
        body: dict[str, Any] = {"sandbox": sandbox}
        if snapshot_name is not None:
            body["name"] = snapshot_name
        data = await self._request("POST", "/snapshots", json=body)
        return SnapshotMeta(**data)

    async def get_snapshot(self, name: str) -> SnapshotMeta:
        """Get info about a snapshot."""
        data = await self._request("GET", f"/snapshots/{name}")
        return SnapshotMeta(**data)

    async def delete_snapshot(self, name: str) -> None:
        """Delete a snapshot."""
        await self._request("DELETE", f"/snapshots/{name}")

    async def restore_snapshot(self, name: str) -> SandboxInfo:
        """Restore a sandbox from a snapshot."""
        data = await self._request("POST", f"/snapshots/{name}/restore")
        return SandboxInfo(**data)

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
        await self.create_sandbox(
            name, image=image, vcpus=vcpus, memory_mb=memory_mb, profile=profile
        )
        return AsyncSandboxSession(name, self)

    async def browser(
        self,
        name: str,
        *,
        memory_mb: int = 2048,
    ) -> AsyncBrowserSession:
        """Create a sandboxed browser session with automatic cleanup.

        Creates a sandbox with Chromium pre-installed. Use ``goto()``,
        ``screenshot()``, and ``evaluate()`` to interact with web pages.

        Example::

            async with await client.browser("my-browser") as browser:
                page = await browser.goto("https://example.com")
                print(page.title, page.links)
            # sandbox auto-removed
        """
        from .browser import _SETUP_CMD, AsyncBrowserSession

        await self.create_sandbox(
            name,
            image="python:3.12-slim",
            memory_mb=memory_mb,
            profile="moderate",
        )
        await self.exec_in_sandbox(name, _SETUP_CMD)
        return AsyncBrowserSession(name, self)

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
