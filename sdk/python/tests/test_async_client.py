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


class TestAsyncDurableOrchestrations:
    async def test_list_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": []})
        async with make_client() as client:
            await client.list_orchestrations()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations"

    async def test_create_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "orch"}})
        async with make_client() as client:
            await client.create_orchestration({"foo": "bar"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations"

    async def test_get_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "orch"}})
        async with make_client() as client:
            await client.get_orchestration("orch-1")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/orch-1"

    async def test_signal_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"accepted": True}})
        async with make_client() as client:
            await client.signal_orchestration("orch-1", {"name": "approval"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/orch-1/events"

    async def test_terminate_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(
            json={"success": True, "data": {"id": "orch-1", "status": "terminated"}},
        )
        async with make_client() as client:
            await client.terminate_orchestration("orch-1", {"reason": "manual"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/orch-1/terminate"

    async def test_list_definitions_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": []})
        async with make_client() as client:
            await client.list_orchestration_definitions()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/definitions"

    async def test_upsert_definition_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"name": "deploy-pipeline"}})
        async with make_client() as client:
            await client.upsert_orchestration_definition({"name": "deploy-pipeline"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/definitions"

    async def test_get_definition_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"name": "deploy-pipeline"}})
        async with make_client() as client:
            await client.get_orchestration_definition("deploy-pipeline")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/definitions/deploy-pipeline"

    async def test_delete_definition_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": "deleted"})
        async with make_client() as client:
            await client.delete_orchestration_definition("deploy-pipeline")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/definitions/deploy-pipeline"


class TestAsyncDurableObjects:
    async def test_list_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": []})
        async with make_client() as client:
            await client.list_objects()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/objects"

    async def test_create_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "obj"}})
        async with make_client() as client:
            await client.create_object({"foo": "bar"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/objects"

    async def test_get_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "obj"}})
        async with make_client() as client:
            await client.get_object("obj-1")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/objects/obj-1"


class TestAsyncDurableSchedules:
    async def test_list_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": []})
        async with make_client() as client:
            await client.list_schedules()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/schedules"

    async def test_create_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "sched"}})
        async with make_client() as client:
            await client.create_schedule({"foo": "bar"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/schedules"

    async def test_get_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "sched"}})
        async with make_client() as client:
            await client.get_schedule("sched-1")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/schedules/sched-1"


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
