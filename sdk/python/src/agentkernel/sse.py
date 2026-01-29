"""SSE stream parsing for the agentkernel SDK."""

from __future__ import annotations

import json
from collections.abc import AsyncIterator, Iterator
from typing import TYPE_CHECKING

import httpx
import httpx_sse

from .types import StreamEvent

if TYPE_CHECKING:
    pass

KNOWN_EVENTS = frozenset({"started", "progress", "output", "done", "error"})


def iter_sse_sync(response: httpx.Response) -> Iterator[StreamEvent]:
    """Parse SSE events from a sync httpx response."""
    with httpx_sse.connect_sse(response.stream) as event_source:  # type: ignore[arg-type]
        for sse in event_source.iter_sse():
            if sse.event not in KNOWN_EVENTS:
                continue
            try:
                data = json.loads(sse.data)
            except (json.JSONDecodeError, TypeError):
                data = {"raw": sse.data}
            event = StreamEvent(type=sse.event, data=data)  # type: ignore[arg-type]
            yield event
            if event.type in ("done", "error"):
                return


async def iter_sse_async(response: httpx.Response) -> AsyncIterator[StreamEvent]:
    """Parse SSE events from an async httpx response."""
    async with httpx_sse.aconnect_sse(response.stream) as event_source:  # type: ignore[arg-type]
        async for sse in event_source.aiter_sse():
            if sse.event not in KNOWN_EVENTS:
                continue
            try:
                data = json.loads(sse.data)
            except (json.JSONDecodeError, TypeError):
                data = {"raw": sse.data}
            event = StreamEvent(type=sse.event, data=data)  # type: ignore[arg-type]
            yield event
            if event.type in ("done", "error"):
                return
