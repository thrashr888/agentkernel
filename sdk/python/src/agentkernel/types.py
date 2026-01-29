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


class RunOptions(BaseModel):
    """Options for the run command."""

    image: str | None = None
    profile: SecurityProfile | None = None
    fast: bool = True


class CreateSandboxOptions(BaseModel):
    """Options for creating a sandbox."""

    image: str | None = None


class StreamEvent(BaseModel):
    """SSE stream event."""

    type: StreamEventType
    data: dict
