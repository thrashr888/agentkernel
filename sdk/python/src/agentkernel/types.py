"""Type definitions for the agentkernel SDK."""

from __future__ import annotations

from typing import Literal

from pydantic import BaseModel


SecurityProfile = Literal["permissive", "moderate", "restrictive"]
SandboxStatus = Literal["running", "stopped"]
StreamEventType = Literal["started", "progress", "output", "done", "error"]


class RunOutput(BaseModel):
    """Output from a command execution."""

    output: str


class SandboxInfo(BaseModel):
    """Information about a sandbox."""

    name: str
    status: SandboxStatus
    backend: str
    image: str | None = None
    vcpus: int | None = None
    memory_mb: int | None = None
    created_at: str | None = None


class RunOptions(BaseModel):
    """Options for the run command."""

    image: str | None = None
    profile: SecurityProfile | None = None
    fast: bool = True


class CreateSandboxOptions(BaseModel):
    """Options for creating a sandbox."""

    image: str | None = None
    vcpus: int | None = None
    memory_mb: int | None = None
    profile: SecurityProfile | None = None
    source_url: str | None = None
    source_ref: str | None = None
    volumes: list[str] | None = None
    """Volume mounts (slug:/path or slug:/path:ro). Create volumes via CLI first."""


class ExecOptions(BaseModel):
    """Options for executing a command in a sandbox."""

    env: list[str] = []
    workdir: str | None = None
    sudo: bool | None = None


class StreamEvent(BaseModel):
    """SSE stream event."""

    type: StreamEventType
    data: dict


class FileReadResponse(BaseModel):
    """Response from reading a file."""

    content: str
    encoding: str
    size: int


class BatchCommand(BaseModel):
    """A command for batch execution."""

    command: list[str]


class BatchResult(BaseModel):
    """Result of a single batch command."""

    output: str | None = None
    error: str | None = None


class BatchRunResponse(BaseModel):
    """Response from batch execution."""

    results: list[BatchResult]


class BatchFileWriteResponse(BaseModel):
    """Result of a batch file write."""

    written: int


DetachedStatus = Literal["running", "completed", "failed"]


class DetachedCommand(BaseModel):
    """A detached (background) command running in a sandbox."""

    id: str
    sandbox: str
    command: list[str]
    pid: int
    status: DetachedStatus
    exit_code: int | None = None
    started_at: str


class DetachedLogsResponse(BaseModel):
    """Response from detached command logs."""

    stdout: str | None = None
    stderr: str | None = None


class ExtendTtlResponse(BaseModel):
    """Response from extending sandbox TTL."""

    expires_at: str | None = None


class SnapshotMeta(BaseModel):
    """Metadata for a sandbox snapshot."""

    name: str
    sandbox: str
    image_tag: str
    backend: str
    base_image: str | None = None
    vcpus: int | None = None
    memory_mb: int | None = None
    created_at: str


class PageLink(BaseModel):
    """A link extracted from a web page."""

    text: str
    href: str


class PageResult(BaseModel):
    """Result of navigating to a web page."""

    title: str
    url: str
    text: str
    """Page body text (truncated to ~8KB)."""
    links: list[PageLink] = []
    """First 50 links on the page."""
