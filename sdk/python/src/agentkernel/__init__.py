"""agentkernel SDK — run AI coding agents in secure, isolated microVMs."""

from .client import AgentKernel, SandboxSession

# Lazy imports for optional async/browser dependencies (httpx, playwright)
try:
    from .async_client import AsyncAgentKernel, AsyncSandboxSession
except ImportError:
    pass

try:
    from .browser import AsyncBrowserSession, BrowserSession
except (ImportError, SyntaxError):
    pass
from .errors import (
    AgentKernelError,
    AuthError,
    NetworkError,
    NotFoundError,
    ServerError,
    StreamError,
    ValidationError,
)
from .types import (
    BatchCommand,
    BatchFileWriteResponse,
    BatchResult,
    BatchRunResponse,
    AriaSnapshot,
    DurableObject,
    BrowserEvent,
    CreateSandboxOptions,
    DetachedCommand,
    DetachedLogsResponse,
    DetachedStatus,
    ExecOptions,
    ExtendTtlResponse,
    FileReadResponse,
    Orchestration,
    PageLink,
    PageResult,
    RunOptions,
    RunOutput,
    SandboxInfo,
    Schedule,
    SecurityProfile,
    SnapshotMeta,
    StreamEvent,
    StreamEventType,
)

__all__ = [
    "AgentKernel",
    "AsyncAgentKernel",
    "SandboxSession",
    "AsyncSandboxSession",
    "BrowserSession",
    "AsyncBrowserSession",
    "AgentKernelError",
    "AriaSnapshot",
    "AuthError",
    "BrowserEvent",
    "NotFoundError",
    "ValidationError",
    "ServerError",
    "NetworkError",
    "StreamError",
    "BatchCommand",
    "BatchFileWriteResponse",
    "BatchResult",
    "BatchRunResponse",
    "CreateSandboxOptions",
    "Orchestration",
    "DurableObject",
    "Schedule",
    "DetachedCommand",
    "DetachedLogsResponse",
    "DetachedStatus",
    "ExecOptions",
    "ExtendTtlResponse",
    "FileReadResponse",
    "PageLink",
    "PageResult",
    "RunOptions",
    "RunOutput",
    "SandboxInfo",
    "SecurityProfile",
    "SnapshotMeta",
    "StreamEvent",
    "StreamEventType",
]
