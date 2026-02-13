//! HTTP hooks for proxy request/response interception (l8v).
//!
//! Hooks allow external systems to observe and react to proxied requests.
//! Supported targets:
//! - **Webhook**: POST event payload as JSON to a URL
//! - **File**: Append JSON event to a file (JSONL format)
//! - **Audit**: Log to the agentkernel audit log (always enabled)

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// When a hook fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    OnRequest,
    OnResponse,
}

/// Where a hook delivers its payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookTarget {
    Webhook { url: String },
    File { path: String },
    Audit,
}

/// A registered proxy hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyHook {
    pub name: String,
    pub event: HookEvent,
    pub target: HookTarget,
}

/// A proxy event emitted for each proxied request.
#[derive(Debug, Clone, Serialize)]
pub struct ProxyEvent {
    /// RFC3339 timestamp
    pub timestamp: String,
    /// Sandbox that originated the request
    pub sandbox: String,
    /// HTTP method
    pub method: String,
    /// Full request URL
    pub url: String,
    /// Destination host
    pub host: String,
    /// Response status (None for CONNECT tunnels or errors)
    pub status: Option<u16>,
    /// Whether a secret was injected into this request
    pub secret_injected: bool,
    /// Request latency in milliseconds (None if not measured)
    pub latency_ms: Option<u64>,
}

/// Thread-safe hook registry shared across proxy connections.
pub type HookRegistry = Arc<RwLock<HookRegistryInner>>;

/// Inner state for the hook registry.
pub struct HookRegistryInner {
    hooks: HashMap<String, ProxyHook>,
}

impl Default for HookRegistryInner {
    fn default() -> Self {
        Self::new()
    }
}

impl HookRegistryInner {
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    pub fn register(&mut self, hook: ProxyHook) -> Result<()> {
        if hook.name.is_empty() {
            bail!("Hook name cannot be empty");
        }
        if let HookTarget::Webhook { ref url } = hook.target
            && !url.starts_with("http://")
            && !url.starts_with("https://")
        {
            bail!("Webhook URL must start with http:// or https://");
        }
        self.hooks.insert(hook.name.clone(), hook);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.hooks.remove(name).is_some()
    }

    pub fn list(&self) -> Vec<ProxyHook> {
        self.hooks.values().cloned().collect()
    }

    pub fn hooks_for_event(&self, event: &HookEvent) -> Vec<ProxyHook> {
        self.hooks
            .values()
            .filter(|h| h.event == *event)
            .cloned()
            .collect()
    }
}

/// Create a new hook registry, optionally seeded with initial hooks.
pub fn new_registry(initial: Vec<ProxyHook>) -> HookRegistry {
    let mut inner = HookRegistryInner::new();
    for hook in initial {
        let _ = inner.register(hook);
    }
    Arc::new(RwLock::new(inner))
}

/// Dispatch an event to all matching hooks.
///
/// Webhook delivery is best-effort (fire-and-forget). Failures are logged but
/// do not block the proxy.
pub async fn dispatch_hooks(event: &HookEvent, payload: &ProxyEvent, registry: &HookRegistry) {
    // Always log to stderr (audit)
    log_proxy_event(payload);

    let hooks = {
        let r = registry.read().await;
        r.hooks_for_event(event)
    };

    for hook in hooks {
        match &hook.target {
            HookTarget::Audit => {
                // Already logged above
            }
            HookTarget::File { path } => {
                if let Ok(json) = serde_json::to_string(payload) {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                    {
                        let _ = writeln!(f, "{}", json);
                    } else {
                        eprintln!("[proxy:hook] Failed to open file: {}", path);
                    }
                }
            }
            HookTarget::Webhook { url } => {
                let url = url.clone();
                let json = match serde_json::to_string(payload) {
                    Ok(j) => j,
                    Err(_) => continue,
                };
                // Fire-and-forget: don't block the proxy
                tokio::spawn(async move {
                    deliver_webhook(&url, &json).await;
                });
            }
        }
    }
}

/// Log a proxy event to stderr (audit trail).
pub fn log_proxy_event(event: &ProxyEvent) {
    eprintln!(
        "[proxy:audit] {} {} {} -> {} (secret={}, status={:?})",
        event.sandbox, event.method, event.url, event.host, event.secret_injected, event.status,
    );
}

/// POST a JSON payload to a webhook URL. Best-effort, logs errors.
async fn deliver_webhook(url: &str, json: &str) {
    use hyper::body::Bytes;

    // Simple TCP-based HTTP POST (no TLS for webhook URLs starting with http://)
    // For production, this would use a proper HTTP client. For MVP, we use hyper directly.
    let uri: hyper::Uri = match url.parse() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("[proxy:hook] Invalid webhook URL '{}': {}", url, e);
            return;
        }
    };

    let host = match uri.host() {
        Some(h) => h.to_string(),
        None => {
            eprintln!("[proxy:hook] No host in webhook URL: {}", url);
            return;
        }
    };
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("https") {
            443
        } else {
            80
        });
    let addr = format!("{}:{}", host, port);

    let stream = match tokio::net::TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[proxy:hook] Failed to connect to {}: {}", addr, e);
            return;
        }
    };

    let io = hyper_util::rt::TokioIo::new(stream);
    let (mut sender, conn) = match hyper::client::conn::http1::handshake(io).await {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[proxy:hook] HTTP handshake with {} failed: {}", addr, e);
            return;
        }
    };

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("[proxy:hook] Connection error: {}", e);
        }
    });

    let body = http_body_util::Full::new(Bytes::from(json.to_string()));
    let req = match hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .header("host", &host)
        .header("user-agent", "agentkernel-proxy-hook/1.0")
        .body(body)
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[proxy:hook] Failed to build request: {}", e);
            return;
        }
    };

    match sender.send_request(req).await {
        Ok(resp) => {
            if !resp.status().is_success() {
                eprintln!(
                    "[proxy:hook] Webhook {} returned status {}",
                    url,
                    resp.status()
                );
            }
        }
        Err(e) => {
            eprintln!("[proxy:hook] Webhook delivery to {} failed: {}", url, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_hook() {
        let mut registry = HookRegistryInner::new();
        let hook = ProxyHook {
            name: "my-hook".to_string(),
            event: HookEvent::OnRequest,
            target: HookTarget::Audit,
        };
        registry.register(hook).unwrap();
        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn test_register_empty_name_fails() {
        let mut registry = HookRegistryInner::new();
        let hook = ProxyHook {
            name: String::new(),
            event: HookEvent::OnRequest,
            target: HookTarget::Audit,
        };
        assert!(registry.register(hook).is_err());
    }

    #[test]
    fn test_register_invalid_webhook_url() {
        let mut registry = HookRegistryInner::new();
        let hook = ProxyHook {
            name: "bad-url".to_string(),
            event: HookEvent::OnRequest,
            target: HookTarget::Webhook {
                url: "not-a-url".to_string(),
            },
        };
        assert!(registry.register(hook).is_err());
    }

    #[test]
    fn test_remove_hook() {
        let mut registry = HookRegistryInner::new();
        let hook = ProxyHook {
            name: "removable".to_string(),
            event: HookEvent::OnResponse,
            target: HookTarget::File {
                path: "/tmp/hook.jsonl".to_string(),
            },
        };
        registry.register(hook).unwrap();
        assert!(registry.remove("removable"));
        assert!(!registry.remove("removable")); // already removed
        assert_eq!(registry.list().len(), 0);
    }

    #[test]
    fn test_hooks_for_event() {
        let mut registry = HookRegistryInner::new();
        registry
            .register(ProxyHook {
                name: "on-req".to_string(),
                event: HookEvent::OnRequest,
                target: HookTarget::Audit,
            })
            .unwrap();
        registry
            .register(ProxyHook {
                name: "on-resp".to_string(),
                event: HookEvent::OnResponse,
                target: HookTarget::Audit,
            })
            .unwrap();

        let req_hooks = registry.hooks_for_event(&HookEvent::OnRequest);
        assert_eq!(req_hooks.len(), 1);
        assert_eq!(req_hooks[0].name, "on-req");

        let resp_hooks = registry.hooks_for_event(&HookEvent::OnResponse);
        assert_eq!(resp_hooks.len(), 1);
        assert_eq!(resp_hooks[0].name, "on-resp");
    }

    #[test]
    fn test_new_registry_with_initial() {
        let hooks = vec![
            ProxyHook {
                name: "h1".to_string(),
                event: HookEvent::OnRequest,
                target: HookTarget::Audit,
            },
            ProxyHook {
                name: "h2".to_string(),
                event: HookEvent::OnResponse,
                target: HookTarget::Audit,
            },
        ];
        let _registry = new_registry(hooks);
        // registry is Arc<RwLock<...>>, just verify it was created
    }

    #[test]
    fn test_hook_serde_roundtrip() {
        let hook = ProxyHook {
            name: "test-hook".to_string(),
            event: HookEvent::OnRequest,
            target: HookTarget::Webhook {
                url: "http://localhost:8080/hook".to_string(),
            },
        };
        let json = serde_json::to_string(&hook).unwrap();
        let deserialized: ProxyHook = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test-hook");
        assert_eq!(deserialized.event, HookEvent::OnRequest);
    }
}
