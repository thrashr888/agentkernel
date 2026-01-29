"""Tests for the asynchronous AsyncAgentKernel client."""

import pytest
from pytest_httpx import HTTPXMock

from agentkernel import AsyncAgentKernel, NotFoundError, RunOutput, SandboxInfo

BASE_URL = "http://localhost:9999"


def make_client(**kwargs) -> AsyncAgentKernel:
    return AsyncAgentKernel(base_url=BASE_URL, **kwargs)


class TestAsyncHealth:
    async def test_returns_ok(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": "ok"})
        async with make_client() as client:
            assert await client.health() == "ok"


class TestAsyncRun:
    async def test_returns_output(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"output": "hello\n"}})
        async with make_client() as client:
            result = await client.run(["echo", "hello"])
            assert isinstance(result, RunOutput)
            assert result.output == "hello\n"


class TestAsyncListSandboxes:
    async def test_returns_list(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(
            json={
                "success": True,
                "data": [{"name": "sb-1", "status": "running", "backend": "docker"}],
            }
        )
        async with make_client() as client:
            result = await client.list_sandboxes()
            assert len(result) == 1
            assert isinstance(result[0], SandboxInfo)


class TestAsyncGetSandbox:
    async def test_not_found(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(status_code=404, json={"success": False, "error": "Not found"})
        async with make_client() as client:
            with pytest.raises(NotFoundError):
                await client.get_sandbox("missing")


class TestAsyncSandboxSession:
    async def test_auto_removes(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(
            json={"success": True, "data": {"name": "sess", "status": "running", "backend": "docker"}}
        )
        httpx_mock.add_response(json={"success": True, "data": "Sandbox removed"})

        async with make_client() as client:
            async with await client.sandbox("sess") as sb:
                assert sb.name == "sess"
        requests = httpx_mock.get_requests()
        assert requests[-1].method == "DELETE"
