//! OpenCode proxy for `opencode attach`.
//!
//! Runs OpenCode's own server inside an agentkernel sandbox and proxies the
//! `attach` protocol through to it. Uses the built-in `opencode-sandbox`
//! template so the image comes pre-configured with the OpenCode CLI.
//!
//! ## Usage
//!
//! ```bash
//! agentkernel serve
//! opencode attach http://localhost:18888/opencode
//! ```
//!
//! ## How It Works
//!
//! 1. First request to `/opencode/*` triggers sandbox creation from `opencode-sandbox` template
//! 2. `opencode serve` starts inside the sandbox on port 3000
//! 3. All subsequent requests proxy through to the OpenCode server in the sandbox

use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::{Request, Response, StatusCode};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::vmm::VmManager;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

fn full<T: Into<bytes::Bytes>>(chunk: T) -> BoxBody {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

/// The port OpenCode server listens on inside the sandbox.
const OPENCODE_PORT: u16 = 3000;

/// Sandbox name for the OpenCode server.
const SANDBOX_NAME: &str = "opencode-attach";

/// Template used to create the sandbox.
const TEMPLATE_NAME: &str = "opencode-sandbox";

// ============================================================================
// OpenCode State
// ============================================================================

/// Tracks the OpenCode sandbox and its upstream URL.
pub struct OpenCodeState {
    upstream_url: RwLock<Option<String>>,
    provisioning: RwLock<bool>,
    vm_manager: Arc<std::sync::OnceLock<Arc<tokio::sync::RwLock<VmManager>>>>,
}

impl OpenCodeState {
    pub fn new(vm_manager: Arc<std::sync::OnceLock<Arc<tokio::sync::RwLock<VmManager>>>>) -> Self {
        Self {
            upstream_url: RwLock::new(None),
            provisioning: RwLock::new(false),
            vm_manager,
        }
    }

    fn ensure_manager(&self) -> Result<Arc<tokio::sync::RwLock<VmManager>>, String> {
        if let Some(manager) = self.vm_manager.get() {
            return Ok(manager.clone());
        }

        let manager = Arc::new(tokio::sync::RwLock::new(
            VmManager::new().map_err(|error| error.to_string())?,
        ));
        let _ = self.vm_manager.set(manager);
        self.vm_manager
            .get()
            .cloned()
            .ok_or_else(|| "VM manager could not be initialized".to_string())
    }

    /// Get or create the upstream OpenCode server URL.
    async fn get_or_create_upstream(&self) -> Result<String, String> {
        // Fast path: already running
        {
            let url = self.upstream_url.read().await;
            if let Some(ref u) = *url {
                return Ok(u.clone());
            }
        }

        // Acquire provisioning lock
        {
            let mut provisioning = self.provisioning.write().await;
            if *provisioning {
                drop(provisioning);
                // Wait for the other provisioner
                for _ in 0..120 {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let url = self.upstream_url.read().await;
                    if let Some(ref u) = *url {
                        return Ok(u.clone());
                    }
                }
                return Err("Timed out waiting for sandbox provisioning".to_string());
            }
            *provisioning = true;
        }

        let result = self.provision_sandbox().await;

        let mut provisioning = self.provisioning.write().await;
        *provisioning = false;

        result
    }

    /// Provision the sandbox using the opencode-sandbox template.
    async fn provision_sandbox(&self) -> Result<String, String> {
        let mgr = self.ensure_manager()?;

        // Resolve the built-in template
        let resolved = crate::template::resolve(TEMPLATE_NAME)
            .map_err(|e| format!("Failed to resolve template '{}': {}", TEMPLATE_NAME, e))?;
        let cfg = resolved
            .parse()
            .map_err(|e| format!("Failed to parse template: {}", e))?;

        let image = cfg.docker_image();
        let vcpus = cfg.resources.vcpus;
        let memory_mb = cfg.resources.memory_mb;

        eprintln!(
            "[opencode] Creating sandbox from '{}' template (image: {})...",
            TEMPLATE_NAME, image
        );

        let mut m = mgr.write().await;

        // Clean up any existing sandbox
        let _ = m.remove(SANDBOX_NAME).await;

        // Create sandbox with template settings
        m.create(SANDBOX_NAME, &image, vcpus, memory_mb)
            .await
            .map_err(|e| format!("Failed to create sandbox: {}", e))?;

        // Apply init_script from template (installs opencode + deps on first start)
        if let Some(ref script) = cfg.sandbox.init_script {
            m.set_init_script(SANDBOX_NAME, script)
                .map_err(|e| format!("Failed to set init script: {}", e))?;
        }

        // Start — init_script runs automatically
        let perms = crate::permissions::SecurityProfile::Moderate.permissions();
        m.start_with_permissions(SANDBOX_NAME, &perms)
            .await
            .map_err(|e| {
                let _ = futures::executor::block_on(m.remove(SANDBOX_NAME));
                format!("Failed to start sandbox: {}", e)
            })?;

        eprintln!(
            "[opencode] Sandbox started, launching OpenCode server on port {}...",
            OPENCODE_PORT
        );

        // Start opencode serve in the background (installer puts binary in ~/.opencode/bin)
        let serve_cmd = vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "export PATH=\"$HOME/.opencode/bin:/usr/local/bin:$PATH\"; nohup opencode serve --port {} --hostname 0.0.0.0 > /tmp/opencode.log 2>&1 &",
                OPENCODE_PORT
            ),
        ];
        let _ = m.exec_cmd(SANDBOX_NAME, &serve_cmd).await;

        // Get the container IP
        let ip = m
            .get_container_ip(SANDBOX_NAME)
            .ok_or("Failed to get container IP")?;

        let upstream = format!("http://{}:{}", ip, OPENCODE_PORT);

        // Release lock while polling for readiness
        drop(m);

        // Wait for OpenCode server to be ready
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        let health_url = format!("{}/global/health", upstream);
        let mut ready = false;
        for attempt in 0..60 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            match client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    ready = true;
                    eprintln!(
                        "[opencode] Server ready at {} (took {}s)",
                        upstream,
                        attempt + 1
                    );
                    break;
                }
                _ => {
                    if attempt % 10 == 9 {
                        eprintln!("[opencode] Still waiting for server... ({}s)", attempt + 1);
                    }
                }
            }
        }

        if !ready {
            // Grab logs for debugging
            let mut m = mgr.write().await;
            if let Ok(logs) = m
                .exec_cmd(
                    SANDBOX_NAME,
                    &[
                        "sh".to_string(),
                        "-c".to_string(),
                        "cat /tmp/opencode.log 2>/dev/null; which opencode 2>/dev/null || echo 'opencode not found'".to_string(),
                    ],
                )
                .await
            {
                eprintln!("[opencode] Debug logs:\n{}", logs);
            }
            let _ = m.remove(SANDBOX_NAME).await;
            return Err("OpenCode server failed to start within 60s".to_string());
        }

        // Store the URL
        let mut url = self.upstream_url.write().await;
        *url = Some(upstream.clone());

        Ok(upstream)
    }
}

impl Default for OpenCodeState {
    fn default() -> Self {
        Self::new(Arc::new(std::sync::OnceLock::new()))
    }
}

// ============================================================================
// Route handler — proxy all requests to OpenCode in the sandbox
// ============================================================================

/// Proxy all `/opencode/*` requests to the OpenCode server running in the sandbox.
pub async fn handle_opencode_request(
    req: Request<Incoming>,
    path_suffix: &str,
    state: Arc<OpenCodeState>,
) -> Response<BoxBody> {
    let upstream = match state.get_or_create_upstream().await {
        Ok(url) => url,
        Err(e) => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &serde_json::json!({"error": format!("Failed to start OpenCode sandbox: {}", e)}),
            );
        }
    };

    // Build upstream URL
    let mut upstream_url = if path_suffix.is_empty() {
        upstream.clone()
    } else {
        format!("{}/{}", upstream, path_suffix)
    };
    if let Some(query) = req.uri().query() {
        upstream_url = format!("{}?{}", upstream_url, query);
    }

    let method = req.method().clone();
    let is_sse = path_suffix == "event"
        || path_suffix.ends_with("/event")
        || req
            .headers()
            .get("accept")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"));

    // Read request body
    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => bytes::Bytes::new(),
    };

    let client = reqwest::Client::builder()
        .timeout(if is_sse {
            std::time::Duration::from_secs(300)
        } else {
            std::time::Duration::from_secs(120)
        })
        .build()
        .unwrap_or_default();

    let mut proxy_req = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        &upstream_url,
    );

    if !body_bytes.is_empty() {
        proxy_req = proxy_req
            .header("content-type", "application/json")
            .body(body_bytes.to_vec());
    }
    if is_sse {
        proxy_req = proxy_req.header("accept", "text/event-stream");
    }

    match proxy_req.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_string();

            if is_sse && content_type.contains("text/event-stream") {
                proxy_sse_response(resp).await
            } else {
                let body = resp.text().await.unwrap_or_else(|_| "{}".to_string());
                Response::builder()
                    .status(status)
                    .header("Content-Type", &content_type)
                    .header("Access-Control-Allow-Origin", "*")
                    .body(full(body))
                    .unwrap()
            }
        }
        Err(e) => json_response(
            StatusCode::BAD_GATEWAY,
            &serde_json::json!({"error": format!("Upstream error: {}", e)}),
        ),
    }
}

/// Stream an SSE response from the upstream OpenCode server.
async fn proxy_sse_response(resp: reqwest::Response) -> Response<BoxBody> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Frame<bytes::Bytes>, hyper::Error>>(64);

    tokio::spawn(async move {
        let mut stream = resp.bytes_stream();
        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    if tx.send(Ok(Frame::data(bytes))).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let body = StreamBody::new(stream).boxed();

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("Access-Control-Allow-Origin", "*")
        .body(body)
        .unwrap()
}

// ============================================================================
// Helpers
// ============================================================================

fn json_response<T: serde::Serialize>(status: StatusCode, data: &T) -> Response<BoxBody> {
    let body = serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .body(full(body))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_state() {
        let state = OpenCodeState::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let url = state.upstream_url.read().await;
            assert!(url.is_none());
        });
    }

    #[test]
    fn test_json_response() {
        let resp = json_response(StatusCode::OK, &serde_json::json!({"test": true}));
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_template_resolves() {
        // Verify the opencode-sandbox template is bundled and parseable
        let resolved = crate::template::resolve(TEMPLATE_NAME);
        assert!(resolved.is_ok(), "opencode-sandbox template should resolve");
        let cfg = resolved.unwrap().parse();
        assert!(cfg.is_ok(), "template should parse");
        let cfg = cfg.unwrap();
        assert_eq!(
            cfg.sandbox.base_image,
            Some("node:24-alpine3.24".to_string())
        );
        assert!(cfg.sandbox.init_script.is_some());
    }
}
