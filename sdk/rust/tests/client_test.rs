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
    let info = client.create_sandbox("test", None, None, None, None).await.unwrap();
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
        .exec_in_sandbox("mybox", &["echo", "hello"])
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
    let err = client.create_sandbox("", None, None, None, None).await.unwrap_err();
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
