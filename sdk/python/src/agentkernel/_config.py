"""Configuration resolution for the agentkernel SDK."""

from __future__ import annotations

import os
from dataclasses import dataclass

DEFAULT_BASE_URL = "http://localhost:18888"
DEFAULT_TIMEOUT = 30.0


@dataclass(frozen=True)
class Config:
    base_url: str
    api_key: str | None
    timeout: float


def resolve_config(
    base_url: str | None = None,
    api_key: str | None = None,
    timeout: float | None = None,
) -> Config:
    """Resolve config from constructor args > env vars > defaults."""
    return Config(
        base_url=(base_url or os.environ.get("AGENTKERNEL_BASE_URL") or DEFAULT_BASE_URL).rstrip("/"),
        api_key=api_key or os.environ.get("AGENTKERNEL_API_KEY"),
        timeout=timeout if timeout is not None else DEFAULT_TIMEOUT,
    )
