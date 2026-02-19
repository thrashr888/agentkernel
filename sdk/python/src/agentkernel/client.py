"""Synchronous client for the agentkernel HTTP API."""

from __future__ import annotations

from collections.abc import Iterator
from typing import Any

import httpx

from ._config import resolve_config
from .errors import AgentKernelError, NetworkError, error_from_status
from .types import (
    BatchFileWriteResponse,
    BatchRunResponse,
    CreateSandboxOptions,
    DetachedCommand,
    DetachedLogsResponse,
    DurableObject,
    DurableStore,
    DurableStoreCommandResult,
    DurableStoreExecuteResult,
    DurableStoreQueryResult,
    ExecOptions,
    ExtendTtlResponse,
    FileReadResponse,
    OrchestrationDefinition,
    Orchestration,
    RunOptions,
    RunOutput,
    SandboxInfo,
    Schedule,
    SecurityProfile,
    SnapshotMeta,
    StreamEvent,
)

SDK_VERSION = "0.3.0"


class SandboxSession:
    """A sandbox session with auto-cleanup on context manager exit."""

    def __init__(self, name: str, client: AgentKernel) -> None:
        self.name = name
        self._client = client
        self._removed = False

    def run(
        self,
        command: list[str],
        *,
        env: list[str] | None = None,
        workdir: str | None = None,
        sudo: bool | None = None,
    ) -> RunOutput:
        """Run a command in this sandbox."""
        return self._client.exec_in_sandbox(
            self.name, command, env=env, workdir=workdir, sudo=sudo,
        )

    def info(self) -> SandboxInfo:
        """Get sandbox info."""
        return self._client.get_sandbox(self.name)

    def write_files(self, files: dict[str, str]) -> BatchFileWriteResponse:
        """Write multiple files in one request."""
        return self._client.write_files(self.name, files)

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
        source_url: str | None = None,
        source_ref: str | None = None,
        volumes: list[str] | None = None,
        secrets: list[str] | None = None,
        secret_files: list[str] | None = None,
    ) -> SandboxInfo:
        """Create a new sandbox."""
        body = {
            "name": name, "image": image, "vcpus": vcpus,
            "memory_mb": memory_mb, "profile": profile,
            "source_url": source_url, "source_ref": source_ref,
            "volumes": volumes,
            "secrets": secrets, "secret_files": secret_files,
        }
        data = self._request(
            "POST",
            "/sandboxes",
            json={k: v for k, v in body.items() if v is not None},
        )
        return SandboxInfo(**data)

    def get_sandbox(self, name: str) -> SandboxInfo:
        """Get info about a sandbox."""
        data = self._request("GET", f"/sandboxes/{name}")
        return SandboxInfo(**data)

    def get_sandbox_by_uuid(self, uuid: str) -> SandboxInfo:
        """Get info about a sandbox by UUID."""
        data = self._request("GET", f"/sandboxes/by-uuid/{uuid}")
        return SandboxInfo(**data)

    def remove_sandbox(self, name: str) -> None:
        """Remove a sandbox."""
        self._request("DELETE", f"/sandboxes/{name}")

    def exec_in_sandbox(
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
        data = self._request("POST", f"/sandboxes/{name}/exec", json=body)
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

    def write_files(self, name: str, files: dict[str, str]) -> BatchFileWriteResponse:
        """Write multiple files to a sandbox in one request."""
        data = self._request("POST", f"/sandboxes/{name}/files", json={"files": files})
        return BatchFileWriteResponse(**data)

    def get_sandbox_logs(self, name: str) -> list[dict]:
        """Get audit log entries for a sandbox."""
        return self._request("GET", f"/sandboxes/{name}/logs")

    def batch_run(self, commands: list[list[str]]) -> BatchRunResponse:
        """Run multiple commands in parallel."""
        batch_commands = [{"command": cmd} for cmd in commands]
        data = self._request("POST", "/batch/run", json={"commands": batch_commands})
        return BatchRunResponse(**data)

    def exec_detached(
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
        data = self._request("POST", f"/sandboxes/{name}/exec/detach", json=body)
        return DetachedCommand(**data)

    def detached_status(self, name: str, cmd_id: str) -> DetachedCommand:
        """Get the status of a detached command."""
        data = self._request("GET", f"/sandboxes/{name}/exec/detached/{cmd_id}")
        return DetachedCommand(**data)

    def detached_logs(
        self, name: str, cmd_id: str, *, stream: str | None = None
    ) -> DetachedLogsResponse:
        """Get logs from a detached command."""
        query = f"?stream={stream}" if stream == "stderr" else ""
        data = self._request(
            "GET", f"/sandboxes/{name}/exec/detached/{cmd_id}/logs{query}"
        )
        return DetachedLogsResponse(**data)

    def detached_kill(self, name: str, cmd_id: str) -> str:
        """Kill a detached command."""
        return self._request("DELETE", f"/sandboxes/{name}/exec/detached/{cmd_id}")

    def detached_list(self, name: str) -> list[DetachedCommand]:
        """List detached commands in a sandbox."""
        data = self._request("GET", f"/sandboxes/{name}/exec/detached")
        return [DetachedCommand(**d) for d in data]

    def list_orchestrations(self) -> list[Orchestration]:
        """List orchestrations."""
        return self._request("GET", "/orchestrations")

    def create_orchestration(self, orchestration: Orchestration) -> Orchestration:
        """Create a new orchestration."""
        return self._request("POST", "/orchestrations", json=orchestration)

    def get_orchestration(self, orchestration_id: str) -> Orchestration:
        """Get an orchestration by identifier."""
        return self._request("GET", f"/orchestrations/{orchestration_id}")

    def signal_orchestration(
        self, orchestration_id: str, event: dict[str, Any],
    ) -> Orchestration:
        """Raise an external event for an orchestration."""
        return self._request(
            "POST", f"/orchestrations/{orchestration_id}/events", json=event,
        )

    def terminate_orchestration(
        self, orchestration_id: str, payload: dict[str, Any] | None = None,
    ) -> Orchestration:
        """Terminate an orchestration."""
        return self._request(
            "POST",
            f"/orchestrations/{orchestration_id}/terminate",
            json=payload or {},
        )

    def list_orchestration_definitions(self) -> list[OrchestrationDefinition]:
        """List orchestration definitions."""
        return self._request("GET", "/orchestrations/definitions")

    def upsert_orchestration_definition(
        self, definition: OrchestrationDefinition,
    ) -> OrchestrationDefinition:
        """Register or update an orchestration definition."""
        return self._request("POST", "/orchestrations/definitions", json=definition)

    def get_orchestration_definition(self, name: str) -> OrchestrationDefinition:
        """Get an orchestration definition by name."""
        return self._request("GET", f"/orchestrations/definitions/{name}")

    def delete_orchestration_definition(self, name: str) -> str:
        """Delete an orchestration definition by name."""
        return self._request("DELETE", f"/orchestrations/definitions/{name}")

    def list_objects(self) -> list[DurableObject]:
        """List objects."""
        return self._request("GET", "/objects")

    def create_object(self, obj: DurableObject) -> DurableObject:
        """Create a new object."""
        return self._request("POST", "/objects", json=obj)

    def get_object(self, object_id: str) -> DurableObject:
        """Get an object by identifier."""
        return self._request("GET", f"/objects/{object_id}")

    def call_object(
        self, class_name: str, object_id: str, method: str, args: dict | None = None,
    ) -> dict:
        """Call a method on a durable object (auto-creates/wakes if needed)."""
        resp = self._request(
            "POST",
            f"/objects/{class_name}/{object_id}/call/{method}",
            json=args or {},
        )
        return resp

    def delete_object(self, object_id: str) -> str:
        """Delete a durable object by identifier."""
        return self._request("DELETE", f"/objects/{object_id}")

    def patch_object(
        self, object_id: str, *, storage: dict | None = None, status: str | None = None,
    ) -> DurableObject:
        """Partially update a durable object (storage and/or status)."""
        body: dict[str, Any] = {}
        if storage is not None:
            body["storage"] = storage
        if status is not None:
            body["status"] = status
        return self._request("PATCH", f"/objects/{object_id}", json=body)

    def list_schedules(self) -> list[Schedule]:
        """List schedules."""
        return self._request("GET", "/schedules")

    def create_schedule(self, schedule: Schedule) -> Schedule:
        """Create a new schedule."""
        return self._request("POST", "/schedules", json=schedule)

    def get_schedule(self, schedule_id: str) -> Schedule:
        """Get a schedule by identifier."""
        return self._request("GET", f"/schedules/{schedule_id}")

    def delete_schedule(self, schedule_id: str) -> str:
        """Delete a schedule by identifier."""
        return self._request("DELETE", f"/schedules/{schedule_id}")

    def list_stores(self) -> list[DurableStore]:
        """List durable stores."""
        return self._request("GET", "/stores")

    def create_store(self, store: DurableStore) -> DurableStore:
        """Create a durable store."""
        return self._request("POST", "/stores", json=store)

    def get_store(self, store_id: str) -> DurableStore:
        """Get a durable store by identifier."""
        return self._request("GET", f"/stores/{store_id}")

    def delete_store(self, store_id: str) -> str:
        """Delete a durable store by identifier."""
        return self._request("DELETE", f"/stores/{store_id}")

    def query_store(
        self, store_id: str, payload: dict[str, Any],
    ) -> DurableStoreQueryResult:
        """Run a read query against a durable store."""
        return self._request("POST", f"/stores/{store_id}/query", json=payload)

    def execute_store(
        self, store_id: str, payload: dict[str, Any],
    ) -> DurableStoreExecuteResult:
        """Run a write statement against a durable store."""
        return self._request("POST", f"/stores/{store_id}/execute", json=payload)

    def command_store(
        self, store_id: str, payload: dict[str, Any],
    ) -> DurableStoreCommandResult:
        """Run a command against a durable store (Redis-style engines)."""
        return self._request("POST", f"/stores/{store_id}/command", json=payload)

    def extend_ttl(self, name: str, *, by: str) -> str | None:
        """Extend a sandbox's TTL. Returns the new expiry time."""
        data = self._request("POST", f"/sandboxes/{name}/extend", json={"by": by})
        return data.get("expires_at") if isinstance(data, dict) else data

    def list_snapshots(self) -> list[SnapshotMeta]:
        """List all snapshots."""
        data = self._request("GET", "/snapshots")
        return [SnapshotMeta(**s) for s in data]

    def take_snapshot(self, sandbox: str, *, snapshot_name: str | None = None) -> SnapshotMeta:
        """Take a snapshot of a sandbox."""
        body: dict[str, Any] = {"sandbox": sandbox}
        if snapshot_name is not None:
            body["name"] = snapshot_name
        data = self._request("POST", "/snapshots", json=body)
        return SnapshotMeta(**data)

    def get_snapshot(self, name: str) -> SnapshotMeta:
        """Get info about a snapshot."""
        data = self._request("GET", f"/snapshots/{name}")
        return SnapshotMeta(**data)

    def delete_snapshot(self, name: str) -> None:
        """Delete a snapshot."""
        self._request("DELETE", f"/snapshots/{name}")

    def restore_snapshot(self, name: str) -> SandboxInfo:
        """Restore a sandbox from a snapshot."""
        data = self._request("POST", f"/snapshots/{name}/restore")
        return SandboxInfo(**data)

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

    def browser(
        self,
        name: str,
        *,
        memory_mb: int = 2048,
    ) -> BrowserSession:
        """Create a sandboxed browser session with automatic cleanup.

        Creates a sandbox with Chromium pre-installed. Use ``goto()``,
        ``screenshot()``, and ``evaluate()`` to interact with web pages.

        Example::

            with client.browser("my-browser") as browser:
                page = browser.goto("https://example.com")
                print(page.title, page.links)
            # sandbox auto-removed
        """
        from .browser import BrowserSession, _SETUP_CMD

        self.create_sandbox(
            name, image="python:3.12-slim", memory_mb=memory_mb, profile="moderate",
        )
        # Install Playwright + Chromium (one-time setup)
        self.exec_in_sandbox(name, _SETUP_CMD)
        return BrowserSession(name, self)

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
