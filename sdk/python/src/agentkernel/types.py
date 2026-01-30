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
