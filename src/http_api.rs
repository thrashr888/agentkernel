//! HTTP API server for agentkernel.
//!
//! Provides RESTful endpoints for sandbox management.
//!
//! ## Authentication
//!
//! API key authentication is optional. To enable:
//! - Set `AGENTKERNEL_API_KEY` environment variable
//! - Or configure `api_key` in the config file
//!
//! When enabled, requests must include the API key in the Authorization header:
//! ```
//! Authorization: Bearer <api_key>
//! ```

use anyhow::Result;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::languages;
use crate::permissions::SecurityProfile;
use crate::validation;
use crate::vmm::VmManager;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

fn full<T: Into<bytes::Bytes>>(chunk: T) -> BoxBody {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

/// Request to run a command
#[derive(Debug, Deserialize)]
struct RunRequest {
    command: Vec<String>,
    image: Option<String>,
    profile: Option<String>,
    /// Use container pool for faster execution (default: true for /run)
    #[serde(default = "default_fast")]
    fast: bool,
}

fn default_fast() -> bool {
    true // Default to fast mode for HTTP API
}

/// Request to create a sandbox
#[derive(Debug, Deserialize)]
struct CreateRequest {
    name: String,
    image: Option<String>,
}

/// Request to execute in a sandbox
#[derive(Debug, Deserialize)]
struct ExecRequest {
    command: Vec<String>,
}

/// API response
#[derive(Debug, Serialize)]
struct ApiResponse<T: Serialize> {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    fn error(msg: impl Into<String>) -> ApiResponse<()> {
        ApiResponse {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

/// Sandbox info for list response
#[derive(Debug, Serialize)]
struct SandboxInfo {
    name: String,
    status: String,
    backend: String,
}

/// Run command response
#[derive(Debug, Serialize)]
struct RunResponse {
    output: String,
}

/// Shared state for the HTTP server
struct AppState {
    /// Optional API key for authentication
    api_key: Option<String>,
}

impl AppState {
    fn new() -> Self {
        // Load API key from environment variable
        let api_key = std::env::var("AGENTKERNEL_API_KEY").ok();
        if api_key.is_some() {
            eprintln!("API key authentication enabled");
        }
        Self { api_key }
    }

    /// Create state with explicit API key
    #[allow(dead_code)]
    fn with_api_key(api_key: Option<String>) -> Self {
        if api_key.is_some() {
            eprintln!("API key authentication enabled");
        }
        Self { api_key }
    }

    async fn get_manager(&self) -> Result<VmManager> {
        VmManager::new()
    }

    /// Check if a request is authenticated
    #[allow(clippy::result_large_err)]
    fn check_auth(&self, req: &Request<Incoming>) -> Result<(), Response<BoxBody>> {
        // If no API key is configured, allow all requests
        let api_key = match &self.api_key {
            Some(key) => key,
            None => return Ok(()),
        };

        // Get Authorization header
        let auth_header = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        match auth_header {
            Some(header) if header.starts_with("Bearer ") => {
                let token = &header[7..];
                if token == api_key {
                    Ok(())
                } else {
                    Err(json_response(
                        StatusCode::UNAUTHORIZED,
                        &ApiResponse::<()>::error("Invalid API key"),
                    ))
                }
            }
            Some(_) => Err(json_response(
                StatusCode::UNAUTHORIZED,
                &ApiResponse::<()>::error("Invalid authorization format. Use: Bearer <api_key>"),
            )),
            None => Err(json_response(
                StatusCode::UNAUTHORIZED,
                &ApiResponse::<()>::error("Missing Authorization header"),
            )),
        }
    }
}

/// Handle HTTP requests
async fn handle_request(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Result<Response<BoxBody>, hyper::Error> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // Parse path segments
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    // Health check doesn't require authentication
    if method == Method::GET && segments.as_slice() == ["health"] {
        return Ok(json_response(StatusCode::OK, &ApiResponse::success("ok")));
    }

    // Check authentication for all other endpoints
    if let Err(resp) = state.check_auth(&req) {
        return Ok(resp);
    }

    let response = match (method, segments.as_slice()) {
        // Run a command in a temporary sandbox
        (Method::POST, ["run"]) => handle_run(req, state).await,

        // Run a command with SSE streaming output
        (Method::POST, ["run", "stream"]) => handle_run_stream(req, state).await,

        // List sandboxes
        (Method::GET, ["sandboxes"]) => handle_list_sandboxes(state).await,

        // Create a sandbox
        (Method::POST, ["sandboxes"]) => handle_create_sandbox(req, state).await,

        // Get sandbox info
        (Method::GET, ["sandboxes", name]) => handle_get_sandbox(name, state).await,

        // Execute in a sandbox
        (Method::POST, ["sandboxes", name, "exec"]) => handle_exec_sandbox(req, name, state).await,

        // Delete a sandbox
        (Method::DELETE, ["sandboxes", name]) => handle_delete_sandbox(name, state).await,

        // 404 for everything else
        _ => json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("Not found"),
        ),
    };

    Ok(response)
}

fn json_response<T: Serialize>(status: StatusCode, data: &T) -> Response<BoxBody> {
    let body = serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(full(body))
        .unwrap()
}

async fn read_json_body<T: for<'de> Deserialize<'de>>(
    req: Request<Incoming>,
) -> Result<T, Response<BoxBody>> {
    let body_bytes = req
        .collect()
        .await
        .map_err(|_| {
            json_response(
                StatusCode::BAD_REQUEST,
                &ApiResponse::<()>::error("Failed to read body"),
            )
        })?
        .to_bytes();

    serde_json::from_slice(&body_bytes).map_err(|e| {
        json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(format!("Invalid JSON: {}", e)),
        )
    })
}

async fn handle_run(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let body: RunRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.command.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("command is required"),
        );
    }

    // Fast path: use container pool (default for HTTP API)
    if body.fast {
        if body.image.is_some() {
            // Pool uses alpine:3.20, warn if custom image requested
            eprintln!("Warning: custom image ignored in fast mode (pool uses alpine:3.20)");
        }

        match VmManager::run_pooled(&body.command).await {
            Ok(output) => {
                return json_response(
                    StatusCode::OK,
                    &ApiResponse::success(RunResponse { output }),
                );
            }
            Err(e) => {
                return json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &ApiResponse::<()>::error(e.to_string()),
                );
            }
        }
    }

    // Slow path: full sandbox lifecycle (when fast=false or custom image needed)

    // Validate Docker image name if provided (security: prevents injection)
    if let Some(ref img) = body.image
        && let Err(e) = validation::validate_docker_image(img)
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let image = body
        .image
        .unwrap_or_else(|| languages::detect_image(&body.command));
    let profile = body.profile.as_deref().unwrap_or("moderate");
    let perms = SecurityProfile::from_str(profile)
        .unwrap_or_default()
        .permissions();

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let sandbox_name = format!("api-run-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Create
    if let Err(e) = manager.create(&sandbox_name, &image, 1, 512).await {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Start
    if let Err(e) = manager.start_with_permissions(&sandbox_name, &perms).await {
        let _ = manager.remove(&sandbox_name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    // Execute
    let result = manager.exec_cmd(&sandbox_name, &body.command).await;

    // Cleanup
    let _ = manager.remove(&sandbox_name).await;

    match result {
        Ok(output) => json_response(
            StatusCode::OK,
            &ApiResponse::success(RunResponse { output }),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

/// Server-Sent Events response for streaming command output
fn sse_response(events: Vec<(&str, serde_json::Value)>) -> Response<BoxBody> {
    let mut body = String::new();
    for (event_type, data) in events {
        body.push_str(&format!(
            "event: {}\ndata: {}\n\n",
            event_type,
            serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string())
        ));
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(full(body))
        .unwrap()
}

/// Handle /run/stream - runs command with SSE streaming output
async fn handle_run_stream(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let body: RunRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.command.is_empty() {
        return sse_response(vec![(
            "error",
            serde_json::json!({"message": "command is required"}),
        )]);
    }

    let mut events = vec![];

    // Send started event
    events.push((
        "started",
        serde_json::json!({
            "command": body.command,
            "fast": body.fast,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }),
    ));

    // Fast path: use container pool (default for HTTP API)
    if body.fast {
        match VmManager::run_pooled(&body.command).await {
            Ok(output) => {
                events.push((
                    "output",
                    serde_json::json!({
                        "data": output,
                        "stream": "stdout"
                    }),
                ));
                events.push((
                    "done",
                    serde_json::json!({
                        "exit_code": 0,
                        "success": true
                    }),
                ));
            }
            Err(e) => {
                events.push((
                    "error",
                    serde_json::json!({
                        "message": e.to_string()
                    }),
                ));
            }
        }
        return sse_response(events);
    }

    // Slow path: full sandbox lifecycle
    let profile = body.profile.as_deref().unwrap_or("moderate");
    let perms = SecurityProfile::from_str(profile)
        .unwrap_or_default()
        .permissions();

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            events.push(("error", serde_json::json!({"message": e.to_string()})));
            return sse_response(events);
        }
    };

    let image = body
        .image
        .clone()
        .unwrap_or_else(|| languages::detect_image(&body.command));

    let sandbox_name = format!("api-stream-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Create
    if let Err(e) = manager.create(&sandbox_name, &image, 1, 512).await {
        events.push(("error", serde_json::json!({"message": e.to_string()})));
        return sse_response(events);
    }

    events.push((
        "progress",
        serde_json::json!({
            "stage": "sandbox_created",
            "sandbox": sandbox_name
        }),
    ));

    // Start
    if let Err(e) = manager.start_with_permissions(&sandbox_name, &perms).await {
        let _ = manager.remove(&sandbox_name).await;
        events.push(("error", serde_json::json!({"message": e.to_string()})));
        return sse_response(events);
    }

    events.push((
        "progress",
        serde_json::json!({
            "stage": "sandbox_started"
        }),
    ));

    // Execute
    let result = manager.exec_cmd(&sandbox_name, &body.command).await;

    // Cleanup
    let _ = manager.remove(&sandbox_name).await;

    match result {
        Ok(output) => {
            events.push((
                "output",
                serde_json::json!({
                    "data": output,
                    "stream": "stdout"
                }),
            ));
            events.push((
                "done",
                serde_json::json!({
                    "exit_code": 0,
                    "success": true
                }),
            ));
        }
        Err(e) => {
            events.push((
                "error",
                serde_json::json!({
                    "message": e.to_string()
                }),
            ));
        }
    }

    sse_response(events)
}

async fn handle_list_sandboxes(state: Arc<AppState>) -> Response<BoxBody> {
    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let sandboxes: Vec<SandboxInfo> = manager
        .list()
        .into_iter()
        .map(|(name, running, backend)| SandboxInfo {
            name: name.to_string(),
            status: if running { "running" } else { "stopped" }.to_string(),
            backend: backend
                .map(|b| format!("{}", b))
                .unwrap_or_else(|| "unknown".to_string()),
        })
        .collect();

    json_response(StatusCode::OK, &ApiResponse::success(sandboxes))
}

async fn handle_create_sandbox(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let body: CreateRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(&body.name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let image = body.image.as_deref().unwrap_or("alpine:3.20");

    // Validate Docker image name if provided
    if let Some(ref img) = body.image
        && let Err(e) = validation::validate_docker_image(img)
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    if let Err(e) = manager.create(&body.name, image, 1, 512).await {
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    if let Err(e) = manager.start(&body.name).await {
        let _ = manager.remove(&body.name).await;
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    json_response(
        StatusCode::CREATED,
        &ApiResponse::success(SandboxInfo {
            name: body.name,
            status: "running".to_string(),
            backend: format!("{}", manager.backend()),
        }),
    )
}

async fn handle_get_sandbox(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    let sandboxes = manager.list();
    for (sandbox_name, running, backend) in sandboxes {
        if sandbox_name == name {
            return json_response(
                StatusCode::OK,
                &ApiResponse::success(SandboxInfo {
                    name: sandbox_name.to_string(),
                    status: if running { "running" } else { "stopped" }.to_string(),
                    backend: backend
                        .map(|b| format!("{}", b))
                        .unwrap_or_else(|| "unknown".to_string()),
                }),
            );
        }
    }

    json_response(
        StatusCode::NOT_FOUND,
        &ApiResponse::<()>::error("Sandbox not found"),
    )
}

async fn handle_exec_sandbox(
    req: Request<Incoming>,
    name: &str,
    state: Arc<AppState>,
) -> Response<BoxBody> {
    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let body: ExecRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    if body.command.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error("command is required"),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.exec_cmd(name, &body.command).await {
        Ok(output) => json_response(
            StatusCode::OK,
            &ApiResponse::success(RunResponse { output }),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

async fn handle_delete_sandbox(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
    // Validate sandbox name (security: prevents command injection)
    if let Err(e) = validation::validate_sandbox_name(name) {
        return json_response(
            StatusCode::BAD_REQUEST,
            &ApiResponse::<()>::error(e.to_string()),
        );
    }

    let mut manager = match state.get_manager().await {
        Ok(m) => m,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ApiResponse::<()>::error(e.to_string()),
            );
        }
    };

    match manager.remove(name).await {
        Ok(_) => json_response(StatusCode::OK, &ApiResponse::success("Sandbox removed")),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &ApiResponse::<()>::error(e.to_string()),
        ),
    }
}

/// Run the HTTP API server
pub async fn run_server(addr: SocketAddr) -> Result<()> {
    let state = Arc::new(AppState::new());
    let listener = TcpListener::bind(addr).await?;

    eprintln!("agentkernel HTTP API server listening on http://{}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let state = state.clone();

        tokio::task::spawn(async move {
            let service = service_fn(move |req| {
                let state = state.clone();
                handle_request(req, state)
            });

            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === ApiResponse tests ===

    #[test]
    fn test_api_response_success() {
        let response = ApiResponse::success("test data");
        assert!(response.success);
        assert_eq!(response.data, Some("test data"));
        assert!(response.error.is_none());
    }

    #[test]
    fn test_api_response_error() {
        let response = ApiResponse::<()>::error("test error");
        assert!(!response.success);
        assert!(response.data.is_none());
        assert_eq!(response.error, Some("test error".to_string()));
    }

    #[test]
    fn test_api_response_success_serialization() {
        let response = ApiResponse::success("data");
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"data\":\"data\""));
        assert!(!json.contains("\"error\"")); // error is skipped when None
    }

    #[test]
    fn test_api_response_error_serialization() {
        let response = ApiResponse::<()>::error("failed");
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":false"));
        assert!(!json.contains("\"data\"")); // data is skipped when None
        assert!(json.contains("\"error\":\"failed\""));
    }

    // === Request deserialization tests ===

    #[test]
    fn test_run_request_deserialize() {
        let json = r#"{"command": ["echo", "hello"], "image": "alpine:3.20"}"#;
        let req: RunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.command, vec!["echo", "hello"]);
        assert_eq!(req.image, Some("alpine:3.20".to_string()));
        assert!(req.fast); // default is true
    }

    #[test]
    fn test_run_request_deserialize_minimal() {
        let json = r#"{"command": ["ls"]}"#;
        let req: RunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.command, vec!["ls"]);
        assert!(req.image.is_none());
        assert!(req.profile.is_none());
        assert!(req.fast);
    }

    #[test]
    fn test_run_request_deserialize_fast_false() {
        let json = r#"{"command": ["ls"], "fast": false}"#;
        let req: RunRequest = serde_json::from_str(json).unwrap();
        assert!(!req.fast);
    }

    #[test]
    fn test_create_request_deserialize() {
        let json = r#"{"name": "my-sandbox", "image": "python:3.12"}"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "my-sandbox");
        assert_eq!(req.image, Some("python:3.12".to_string()));
    }

    #[test]
    fn test_create_request_deserialize_minimal() {
        let json = r#"{"name": "my-sandbox"}"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "my-sandbox");
        assert!(req.image.is_none());
    }

    #[test]
    fn test_exec_request_deserialize() {
        let json = r#"{"command": ["npm", "test"]}"#;
        let req: ExecRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.command, vec!["npm", "test"]);
    }

    // === SandboxInfo tests ===

    #[test]
    fn test_sandbox_info_serialize() {
        let info = SandboxInfo {
            name: "test-sandbox".to_string(),
            status: "running".to_string(),
            backend: "docker".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"test-sandbox\""));
        assert!(json.contains("\"status\":\"running\""));
    }

    // === RunResponse tests ===

    #[test]
    fn test_run_response_serialize() {
        let response = RunResponse {
            output: "hello world".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"output\":\"hello world\""));
    }

    // === AppState tests ===

    #[test]
    fn test_app_state_with_api_key() {
        let state = AppState::with_api_key(Some("secret123".to_string()));
        assert_eq!(state.api_key, Some("secret123".to_string()));
    }

    #[test]
    fn test_app_state_without_api_key() {
        let state = AppState::with_api_key(None);
        assert!(state.api_key.is_none());
    }

    // === default_fast tests ===

    #[test]
    fn test_default_fast_returns_true() {
        assert!(default_fast());
    }

    // === json_response tests ===

    #[test]
    fn test_json_response_ok() {
        let response = json_response(StatusCode::OK, &ApiResponse::success("data"));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_json_response_not_found() {
        let response = json_response(
            StatusCode::NOT_FOUND,
            &ApiResponse::<()>::error("not found"),
        );
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_json_response_created() {
        let info = SandboxInfo {
            name: "test".to_string(),
            status: "running".to_string(),
            backend: "docker".to_string(),
        };
        let response = json_response(StatusCode::CREATED, &ApiResponse::success(info));
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    // === Path parsing tests (unit test the segment logic) ===

    #[test]
    fn test_path_segments_parsing() {
        let path = "/sandboxes/my-sandbox/exec";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["sandboxes", "my-sandbox", "exec"]);
    }

    #[test]
    fn test_path_segments_health() {
        let path = "/health";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["health"]);
    }

    #[test]
    fn test_path_segments_run() {
        let path = "/run";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["run"]);
    }

    #[test]
    fn test_path_segments_sandboxes() {
        let path = "/sandboxes";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["sandboxes"]);
    }

    #[test]
    fn test_path_segments_sandbox_by_name() {
        let path = "/sandboxes/test-123";
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        assert_eq!(segments, vec!["sandboxes", "test-123"]);
    }
}
