"""Tests for the synchronous AgentKernel client."""

from typing import Any

import pytest
from pytest_httpx import HTTPXMock

from agentkernel import (
    AgentKernel,
    AuthError,
    NotFoundError,
    RunOutput,
    SandboxInfo,
    ServerError,
    ValidationError,
)

BASE_URL = "http://localhost:9999"


def make_client(**kwargs: Any) -> AgentKernel:
    return AgentKernel(base_url=BASE_URL, **kwargs)


class TestHealth:
    def test_returns_ok(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": "ok"})
        assert make_client().health() == "ok"


class TestRun:
    def test_returns_output(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"output": "hello\n"}})
        result = make_client().run(["echo", "hello"])
        assert isinstance(result, RunOutput)
        assert result.output == "hello\n"

    def test_passes_options(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"output": "ok\n"}})
        make_client().run(
            ["python3", "-c", "print('ok')"],
            image="python:3.12-alpine",
            profile="restrictive",
            fast=False,
        )
        request = httpx_mock.get_request()
        assert request is not None
        import json

        body = json.loads(request.content)
        assert body["image"] == "python:3.12-alpine"
        assert body["profile"] == "restrictive"
        assert body["fast"] is False


class TestListSandboxes:
    def test_returns_list(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(
            json={
                "success": True,
                "data": [
                    {"name": "sb-1", "status": "running", "backend": "docker"},
                    {"name": "sb-2", "status": "stopped", "backend": "docker"},
                ],
            }
        )
        result = make_client().list_sandboxes()
        assert len(result) == 2
        assert all(isinstance(s, SandboxInfo) for s in result)
        assert result[0].name == "sb-1"


class TestCreateSandbox:
    def test_creates(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(
            status_code=201,
            json={
                "success": True,
                "data": {"name": "new", "status": "running", "backend": "docker"},
            },
        )
        result = make_client().create_sandbox("new")
        assert result.name == "new"
        assert result.status == "running"


class TestGetSandbox:
    def test_returns_info(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(
            json={
                "success": True,
                "data": {"name": "test", "status": "running", "backend": "docker"},
            }
        )
        result = make_client().get_sandbox("test")
        assert result.name == "test"

    def test_not_found(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(status_code=404, json={"success": False, "error": "Not found"})
        with pytest.raises(NotFoundError):
            make_client().get_sandbox("missing")


class TestDurableOrchestrations:
    def test_list_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": []})
        make_client().list_orchestrations()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations"

    def test_create_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "orch"}})
        make_client().create_orchestration({"foo": "bar"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations"

    def test_get_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "orch"}})
        make_client().get_orchestration("orch-1")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/orch-1"

    def test_signal_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"accepted": True}})
        make_client().signal_orchestration("orch-1", {"name": "approval"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/orch-1/events"

    def test_terminate_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(
            json={"success": True, "data": {"id": "orch-1", "status": "terminated"}},
        )
        make_client().terminate_orchestration("orch-1", {"reason": "manual"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/orch-1/terminate"

    def test_list_definitions_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": []})
        make_client().list_orchestration_definitions()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/definitions"

    def test_upsert_definition_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"name": "deploy-pipeline"}})
        make_client().upsert_orchestration_definition({"name": "deploy-pipeline"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/definitions"

    def test_get_definition_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"name": "deploy-pipeline"}})
        make_client().get_orchestration_definition("deploy-pipeline")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/definitions/deploy-pipeline"

    def test_delete_definition_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": "deleted"})
        make_client().delete_orchestration_definition("deploy-pipeline")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/orchestrations/definitions/deploy-pipeline"


class TestDurableObjects:
    def test_list_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": []})
        make_client().list_objects()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/objects"

    def test_create_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "obj"}})
        make_client().create_object({"foo": "bar"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/objects"

    def test_get_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "obj"}})
        make_client().get_object("obj-1")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/objects/obj-1"


class TestDurableSchedules:
    def test_list_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": []})
        make_client().list_schedules()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/schedules"

    def test_create_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "sched"}})
        make_client().create_schedule({"foo": "bar"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/schedules"

    def test_get_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "sched"}})
        make_client().get_schedule("sched-1")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/schedules/sched-1"


class TestDurableStores:
    def test_list_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": []})
        make_client().list_stores()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/stores"

    def test_create_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "store-1"}})
        make_client().create_store({"name": "store-1", "kind": "sqlite"})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/stores"

    def test_get_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"id": "store-1"}})
        make_client().get_store("store-1")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/stores/store-1"

    def test_delete_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": "deleted"})
        make_client().delete_store("store-1")
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/stores/store-1"

    def test_query_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(
            json={
                "success": True,
                "data": {"columns": ["id"], "rows": [{"id": 1}], "row_count": 1},
            },
        )
        make_client().query_store("store-1", {"sql": "select 1", "params": []})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/stores/store-1/query"

    def test_execute_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(
            json={"success": True, "data": {"rows_affected": 1, "last_insert_rowid": 1}},
        )
        make_client().execute_store(
            "store-1",
            {"sql": "insert into t(v) values (?)", "params": ["x"]},
        )
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/stores/store-1/execute"

    def test_command_path(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"result": "PONG"}})
        make_client().command_store("store-1", {"command": ["PING"]})
        request = httpx_mock.get_request()
        assert request is not None
        assert request.url.path == "/stores/store-1/command"


class TestRemoveSandbox:
    def test_removes(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": "Sandbox removed"})
        make_client().remove_sandbox("test")  # no exception


class TestExecInSandbox:
    def test_returns_output(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": {"output": "result\n"}})
        result = make_client().exec_in_sandbox("test", ["echo", "result"])
        assert result.output == "result\n"


class TestSandboxSession:
    def test_auto_removes(self, httpx_mock: HTTPXMock) -> None:
        # create
        httpx_mock.add_response(
            json={
                "success": True,
                "data": {"name": "sess", "status": "running", "backend": "docker"},
            }
        )
        # remove
        httpx_mock.add_response(json={"success": True, "data": "Sandbox removed"})

        client = make_client()
        with client.sandbox("sess") as sb:
            assert sb.name == "sess"
        # remove was called
        requests = httpx_mock.get_requests()
        assert requests[-1].method == "DELETE"

    def test_remove_is_idempotent(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(
            json={
                "success": True,
                "data": {"name": "idem", "status": "running", "backend": "docker"},
            }
        )
        httpx_mock.add_response(json={"success": True, "data": "Sandbox removed"})

        client = make_client()
        sb = client.sandbox("idem")
        sb.remove()
        sb.remove()  # second call is a no-op
        delete_requests = [r for r in httpx_mock.get_requests() if r.method == "DELETE"]
        assert len(delete_requests) == 1


class TestAuth:
    def test_sends_bearer_token(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": "ok"})
        make_client(api_key="sk-test").health()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.headers["authorization"] == "Bearer sk-test"


class TestErrors:
    def test_401(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(status_code=401, json={"success": False, "error": "Unauthorized"})
        with pytest.raises(AuthError):
            make_client().health()

    def test_400(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(status_code=400, json={"success": False, "error": "Bad request"})
        with pytest.raises(ValidationError):
            make_client().run([])

    def test_500(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(status_code=500, json={"success": False, "error": "Internal error"})
        with pytest.raises(ServerError):
            make_client().health()


class TestUserAgent:
    def test_sends_user_agent(self, httpx_mock: HTTPXMock) -> None:
        httpx_mock.add_response(json={"success": True, "data": "ok"})
        make_client().health()
        request = httpx_mock.get_request()
        assert request is not None
        assert request.headers["user-agent"].startswith("agentkernel-python-sdk/")
