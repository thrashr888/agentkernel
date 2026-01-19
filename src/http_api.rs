//! HTTP API server for agentkernel.
//!
//! Provides RESTful endpoints for sandbox management.

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
}

/// Run command response
#[derive(Debug, Serialize)]
struct RunResponse {
    output: String,
}

/// Shared state for the HTTP server
struct AppState {}

impl AppState {
    fn new() -> Self {
        Self {}
    }

    async fn get_manager(&self) -> Result<VmManager> {
        VmManager::new()
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

    let response = match (method, segments.as_slice()) {
        // Health check
        (Method::GET, ["health"]) => json_response(StatusCode::OK, &ApiResponse::success("ok")),

        // Run a command in a temporary sandbox
        (Method::POST, ["run"]) => handle_run(req, state).await,

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
        .map(|(name, running)| SandboxInfo {
            name: name.to_string(),
            status: if running { "running" } else { "stopped" }.to_string(),
        })
        .collect();

    json_response(StatusCode::OK, &ApiResponse::success(sandboxes))
}

async fn handle_create_sandbox(req: Request<Incoming>, state: Arc<AppState>) -> Response<BoxBody> {
    let body: CreateRequest = match read_json_body(req).await {
        Ok(b) => b,
        Err(resp) => return resp,
    };

    let image = body.image.as_deref().unwrap_or("alpine:3.20");

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
        }),
    )
}

async fn handle_get_sandbox(name: &str, state: Arc<AppState>) -> Response<BoxBody> {
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
    for (sandbox_name, running) in sandboxes {
        if sandbox_name == name {
            return json_response(
                StatusCode::OK,
                &ApiResponse::success(SandboxInfo {
                    name: sandbox_name.to_string(),
                    status: if running { "running" } else { "stopped" }.to_string(),
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
