"""Exception hierarchy for the agentkernel SDK."""

from __future__ import annotations


class AgentKernelError(Exception):
    """Base error for all agentkernel SDK errors."""


class AuthError(AgentKernelError):
    """401 Unauthorized."""

    status = 401


class NotFoundError(AgentKernelError):
    """404 Not Found."""

    status = 404


class ValidationError(AgentKernelError):
    """400 Bad Request."""

    status = 400


class ServerError(AgentKernelError):
    """500 Internal Server Error."""

    status = 500


class NetworkError(AgentKernelError):
    """Network / connection error."""


class StreamError(AgentKernelError):
    """SSE streaming error."""


def error_from_status(status: int, body: str) -> AgentKernelError:
    """Map an HTTP status code + body to the appropriate error."""
    import json

    try:
        parsed = json.loads(body)
        message = parsed.get("error", body)
    except (json.JSONDecodeError, TypeError):
        message = body

    errors = {400: ValidationError, 401: AuthError, 404: NotFoundError}
    cls = errors.get(status, ServerError)
    return cls(message)
