use agentkernel_sdk::{AgentKernel, Error};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn test_client(server: &MockServer) -> AgentKernel {
    AgentKernel::builder()
        .base_url(server.uri())
        .build()
        .unwrap()
}

#[tokio::test]
async fn health_check() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"success": true, "data": "ok"})),
        )
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let result = client.health().await.unwrap();
    assert_eq!(result, "ok");
}

#[tokio::test]
async fn run_command() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/run"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"output": "hello world"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let result = client.run(&["echo", "hello world"], None).await.unwrap();
    assert_eq!(result.output, "hello world");
}

#[tokio::test]
async fn list_sandboxes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": [
                {"name": "sb1", "status": "running", "backend": "docker"},
                {"name": "sb2", "status": "stopped", "backend": "docker"}
            ]
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let sandboxes = client.list_sandboxes().await.unwrap();
    assert_eq!(sandboxes.len(), 2);
    assert_eq!(sandboxes[0].name, "sb1");
    assert_eq!(sandboxes[1].status, "stopped");
}

#[tokio::test]
async fn create_sandbox() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "success": true,
            "data": {"name": "test", "status": "running", "backend": "docker"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let info = client.create_sandbox("test", None).await.unwrap();
    assert_eq!(info.name, "test");
    assert_eq!(info.status, "running");
}

#[tokio::test]
async fn get_sandbox() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sandboxes/mybox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"name": "mybox", "status": "running", "backend": "firecracker"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let info = client.get_sandbox("mybox").await.unwrap();
    assert_eq!(info.name, "mybox");
    assert_eq!(info.backend, "firecracker");
}

#[tokio::test]
async fn list_orchestrations() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orchestrations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": [{"id": "orch-1"}]
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let list = client.list_orchestrations().await.unwrap();
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn create_orchestration() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orchestrations"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "success": true,
            "data": {"id": "orch-1"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let orch = client
        .create_orchestration(serde_json::json!({"name": "orch-1"}))
        .await
        .unwrap();
    assert_eq!(orch["id"], "orch-1");
}

#[tokio::test]
async fn get_orchestration() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orchestrations/orch-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"id": "orch-1"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let orch = client.get_orchestration("orch-1").await.unwrap();
    assert_eq!(orch["id"], "orch-1");
}

#[tokio::test]
async fn signal_orchestration() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orchestrations/orch-1/events"))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
            "success": true,
            "data": {"accepted": true}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let out = client
        .signal_orchestration("orch-1", serde_json::json!({"name": "approval"}))
        .await
        .unwrap();
    assert_eq!(out["accepted"], true);
}

#[tokio::test]
async fn terminate_orchestration() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orchestrations/orch-1/terminate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"id": "orch-1", "status": "terminated"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let out = client
        .terminate_orchestration("orch-1", Some(serde_json::json!({"reason": "manual"})))
        .await
        .unwrap();
    assert_eq!(out["status"], "terminated");
}

#[tokio::test]
async fn list_orchestration_definitions() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orchestrations/definitions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": [{"name": "deploy-pipeline"}]
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let out = client.list_orchestration_definitions().await.unwrap();
    assert_eq!(out.len(), 1);
}

#[tokio::test]
async fn upsert_orchestration_definition() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/orchestrations/definitions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"name": "deploy-pipeline"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let out = client
        .upsert_orchestration_definition(serde_json::json!({"name": "deploy-pipeline"}))
        .await
        .unwrap();
    assert_eq!(out["name"], "deploy-pipeline");
}

#[tokio::test]
async fn get_orchestration_definition() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orchestrations/definitions/deploy-pipeline"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"name": "deploy-pipeline"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let out = client
        .get_orchestration_definition("deploy-pipeline")
        .await
        .unwrap();
    assert_eq!(out["name"], "deploy-pipeline");
}

#[tokio::test]
async fn delete_orchestration_definition() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/orchestrations/definitions/deploy-pipeline"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": "deleted"
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let out = client
        .delete_orchestration_definition("deploy-pipeline")
        .await
        .unwrap();
    assert_eq!(out, "deleted");
}

#[tokio::test]
async fn list_objects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/objects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": [{"id": "obj-1"}]
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let list = client.list_objects().await.unwrap();
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn create_object() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/objects"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "success": true,
            "data": {"id": "obj-1"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let obj = client
        .create_object(serde_json::json!({"name": "obj-1"}))
        .await
        .unwrap();
    assert_eq!(obj["id"], "obj-1");
}

#[tokio::test]
async fn get_object() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/objects/obj-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"id": "obj-1"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let obj = client.get_object("obj-1").await.unwrap();
    assert_eq!(obj["id"], "obj-1");
}

#[tokio::test]
async fn list_schedules() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/schedules"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": [{"id": "sched-1"}]
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let list = client.list_schedules().await.unwrap();
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn create_schedule() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/schedules"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "success": true,
            "data": {"id": "sched-1"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let sched = client
        .create_schedule(serde_json::json!({"name": "sched-1"}))
        .await
        .unwrap();
    assert_eq!(sched["id"], "sched-1");
}

#[tokio::test]
async fn get_schedule() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/schedules/sched-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"id": "sched-1"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let sched = client.get_schedule("sched-1").await.unwrap();
    assert_eq!(sched["id"], "sched-1");
}

#[tokio::test]
async fn list_stores() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/stores"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": [{"id": "store-1"}]
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let list = client.list_stores().await.unwrap();
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn create_store() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/stores"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "success": true,
            "data": {"id": "store-1"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let store = client
        .create_store(serde_json::json!({"name": "store-1", "kind": "sqlite"}))
        .await
        .unwrap();
    assert_eq!(store["id"], "store-1");
}

#[tokio::test]
async fn get_store() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/stores/store-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"id": "store-1"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let store = client.get_store("store-1").await.unwrap();
    assert_eq!(store["id"], "store-1");
}

#[tokio::test]
async fn delete_store() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/stores/store-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": "deleted"
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let out = client.delete_store("store-1").await.unwrap();
    assert_eq!(out, "deleted");
}

#[tokio::test]
async fn query_store() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/stores/store-1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"columns": ["id"], "rows": [{"id": 1}], "row_count": 1}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let out = client
        .query_store(
            "store-1",
            serde_json::json!({"sql": "select 1", "params": []}),
        )
        .await
        .unwrap();
    assert_eq!(out["row_count"], 1);
}

#[tokio::test]
async fn execute_store() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/stores/store-1/execute"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"rows_affected": 1, "last_insert_rowid": 1}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let out = client
        .execute_store(
            "store-1",
            serde_json::json!({"sql": "insert into t(v) values (?)", "params": ["x"]}),
        )
        .await
        .unwrap();
    assert_eq!(out["rows_affected"], 1);
}

#[tokio::test]
async fn command_store() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/stores/store-1/command"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"result": "PONG"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let out = client
        .command_store("store-1", serde_json::json!({"command": ["PING"]}))
        .await
        .unwrap();
    assert_eq!(out["result"], "PONG");
}

#[tokio::test]
async fn remove_sandbox() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/sandboxes/mybox"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": "Sandbox removed"
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    client.remove_sandbox("mybox").await.unwrap();
}

#[tokio::test]
async fn exec_in_sandbox() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandboxes/mybox/exec"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"output": "executed"}
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let result = client
        .exec_in_sandbox("mybox", &["echo", "hello"], None)
        .await
        .unwrap();
    assert_eq!(result.output, "executed");
}

#[tokio::test]
async fn error_401() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": "Invalid API key"
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let err = client.health().await.unwrap_err();
    assert!(matches!(err, Error::Auth(_)));
}

#[tokio::test]
async fn error_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/sandboxes/nope"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": "Sandbox not found"
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let err = client.get_sandbox("nope").await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}

#[tokio::test]
async fn error_400() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "Name required"
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let err = client.create_sandbox("", None).await.unwrap_err();
    assert!(matches!(err, Error::Validation(_)));
}

#[tokio::test]
async fn error_500() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
            "error": "Internal error"
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let err = client.health().await.unwrap_err();
    assert!(matches!(err, Error::Server(_)));
}

#[tokio::test]
async fn user_agent_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .and(header(
            "user-agent",
            &format!("agentkernel-rust-sdk/{}", env!("CARGO_PKG_VERSION")),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"success": true, "data": "ok"})),
        )
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let result = client.health().await.unwrap();
    assert_eq!(result, "ok");
}

#[tokio::test]
async fn api_key_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .and(header("authorization", "Bearer sk-test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"success": true, "data": "ok"})),
        )
        .mount(&server)
        .await;

    let client = AgentKernel::builder()
        .base_url(server.uri())
        .api_key("sk-test-key")
        .build()
        .unwrap();
    let result = client.health().await.unwrap();
    assert_eq!(result, "ok");
}

#[tokio::test]
async fn with_sandbox_cleanup() {
    let server = MockServer::start().await;

    // Create sandbox
    Mock::given(method("POST"))
        .and(path("/sandboxes"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "success": true,
            "data": {"name": "guard-test", "status": "running", "backend": "docker"}
        })))
        .expect(1)
        .mount(&server)
        .await;

    // Exec in sandbox
    Mock::given(method("POST"))
        .and(path("/sandboxes/guard-test/exec"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": {"output": "done"}
        })))
        .expect(1)
        .mount(&server)
        .await;

    // Delete sandbox (cleanup)
    Mock::given(method("DELETE"))
        .and(path("/sandboxes/guard-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "data": "Sandbox removed"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    client
        .with_sandbox("guard-test", None, |sb| async move {
            let result = sb.run(&["echo", "test"]).await?;
            assert_eq!(result.output, "done");
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn api_failure_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": false,
            "error": "Service degraded"
        })))
        .mount(&server)
        .await;

    let client = test_client(&server).await;
    let err = client.health().await.unwrap_err();
    assert!(matches!(err, Error::Server(_)));
    assert!(err.to_string().contains("Service degraded"));
}
