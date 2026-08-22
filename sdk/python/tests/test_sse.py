"""Contract tests for parsing an already-open httpx SSE response."""

from collections.abc import AsyncIterator

import httpx

from agentkernel.sse import iter_sse_async, iter_sse_sync

SSE_BODY = b"event: started\ndata: {\"phase\":\"boot\"}\n\nevent: done\ndata: {\"ok\":true}\n\n"
SSE_HEADERS = {"content-type": "text/event-stream"}


class AsyncSSEStream(httpx.AsyncByteStream):
    async def __aiter__(self) -> AsyncIterator[bytes]:
        yield SSE_BODY


def test_iter_sse_sync_uses_existing_response_stream() -> None:
    response = httpx.Response(200, headers=SSE_HEADERS, content=SSE_BODY)

    events = list(iter_sse_sync(response))

    assert [event.type for event in events] == ["started", "done"]
    assert events[0].data == {"phase": "boot"}
    assert events[1].data == {"ok": True}


async def test_iter_sse_async_uses_existing_response_stream() -> None:
    response = httpx.Response(200, headers=SSE_HEADERS, stream=AsyncSSEStream())

    events = [event async for event in iter_sse_async(response)]

    assert [event.type for event in events] == ["started", "done"]
    assert events[0].data == {"phase": "boot"}
    assert events[1].data == {"ok": True}
