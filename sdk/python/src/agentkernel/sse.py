"""SSE stream parsing for the agentkernel SDK."""

from __future__ import annotations

import json
from collections.abc import AsyncIterator, Iterator
from typing import cast

import httpx
import httpx_sse

from .types import StreamEvent, StreamEventType

KNOWN_EVENTS = frozenset({"started", "progress", "output", "done", "error"})


def iter_sse_sync(response: httpx.Response) -> Iterator[StreamEvent]:
    """Parse SSE events from a sync httpx response."""
    event_source = httpx_sse.EventSource(response)
    for sse in event_source.iter_sse():
        if sse.event not in KNOWN_EVENTS:
            continue
        try:
            data = json.loads(sse.data)
        except (json.JSONDecodeError, TypeError):
            data = {"raw": sse.data}
        event = StreamEvent(type=cast("StreamEventType", sse.event), data=data)
        yield event
        if event.type in ("done", "error"):
            return


async def iter_sse_async(response: httpx.Response) -> AsyncIterator[StreamEvent]:
    """Parse SSE events from an async httpx response."""
    event_source = httpx_sse.EventSource(response)
    async for sse in event_source.aiter_sse():
        if sse.event not in KNOWN_EVENTS:
            continue
        try:
            data = json.loads(sse.data)
        except (json.JSONDecodeError, TypeError):
            data = {"raw": sse.data}
        event = StreamEvent(type=cast("StreamEventType", sse.event), data=data)
        yield event
        if event.type in ("done", "error"):
            return
