//! Event bus for sandbox lifecycle events.
//!
//! Produces `SandboxEvent` structs via a `tokio::broadcast` channel.
//! Consumers: webhook dispatcher, SSE endpoint, OTel span emitter.

use chrono::{DateTime, Utc};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::broadcast;

use crate::http_api::BoxBody;

/// A sandbox lifecycle event.
#[derive(Debug, Clone, Serialize)]
pub struct SandboxEvent {
    /// Event type, e.g. "sandbox.created", "sandbox.exec.completed".
    pub event: String,
    /// When the event occurred.
    pub timestamp: DateTime<Utc>,
    /// Sandbox name.
    pub sandbox: String,
    /// Sandbox labels at the time of the event.
    pub labels: HashMap<String, String>,
    /// Arbitrary metadata (exit_code, duration_ms, backend, image, etc.).
    pub metadata: serde_json::Value,
}

/// Broadcast sender half – cloneable, cheap.
pub type EventBus = broadcast::Sender<SandboxEvent>;

/// Create a new event bus with a reasonable buffer.
pub fn new_event_bus() -> EventBus {
    broadcast::channel(256).0
}

// ---------------------------------------------------------------------------
// Webhook dispatcher
// ---------------------------------------------------------------------------

/// Background task that forwards events to webhook URLs with retry.
///
/// Uses a bounded semaphore to limit concurrent in-flight deliveries.
pub async fn webhook_dispatcher(mut rx: broadcast::Receiver<SandboxEvent>, urls: Vec<String>) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    // Limit concurrent webhook deliveries to avoid unbounded task growth
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(64));

    loop {
        match rx.recv().await {
            Ok(event) => {
                for url in &urls {
                    let client = client.clone();
                    let url = url.clone();
                    let payload = event.clone();
                    let permit = semaphore.clone();
                    tokio::spawn(async move {
                        let _permit = permit.acquire().await;
                        for attempt in 0..3u32 {
                            match client.post(&url).json(&payload).send().await {
                                Ok(resp) if resp.status().is_success() => break,
                                Ok(resp) => {
                                    eprintln!(
                                        "[webhook] POST {} returned {} (attempt {})",
                                        url,
                                        resp.status(),
                                        attempt + 1
                                    );
                                }
                                Err(e) => {
                                    eprintln!(
                                        "[webhook] POST {} failed: {} (attempt {})",
                                        url,
                                        e,
                                        attempt + 1
                                    );
                                }
                            }
                            tokio::time::sleep(Duration::from_millis(100 * 2u64.pow(attempt)))
                                .await;
                        }
                    });
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                eprintln!(
                    "[webhook] receiver lagged, skipped {} event(s); continuing",
                    skipped
                );
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// SSE endpoint: GET /events
// ---------------------------------------------------------------------------

fn full_body<T: Into<bytes::Bytes>>(chunk: T) -> BoxBody {
    http_body_util::Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

/// Handle `GET /events` — streams sandbox lifecycle events as SSE.
///
/// Supports optional `?sandbox=<name>` query filter.
pub async fn handle_events_sse(req: &Request<Incoming>, event_bus: &EventBus) -> Response<BoxBody> {
    let query = req.uri().query().unwrap_or("");
    let sandbox_filter: Option<String> = query
        .split('&')
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            match (kv.next(), kv.next()) {
                (Some("sandbox"), Some(v)) => Some(urlencoding::decode(v).ok()?.into_owned()),
                _ => None,
            }
        })
        .next();

    let mut rx = event_bus.subscribe();

    // Collect events for a short window then flush (non-streaming SSE for hyper 1.x).
    // For a long-lived SSE stream we'd need http-body channels; for now we collect
    // for up to 30s or 100 events, whichever comes first.
    let mut body = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut count = 0usize;

    // Send an initial comment so the client knows the stream is alive.
    body.push_str(": connected\n\n");

    loop {
        if count >= 100 {
            break;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(event)) => {
                if let Some(ref filter) = sandbox_filter
                    && event.sandbox != *filter
                {
                    continue;
                }
                let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
                body.push_str(&format!("event: {}\ndata: {}\n\n", event.event, data));
                count += 1;
            }
            Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                body.push_str(&format!(": lagged {n} events\n\n"));
            }
            Ok(Err(_)) => break, // channel closed
            Err(_) => break,     // timeout
        }
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(full_body(body))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Helper: emit an event to the bus (no-op if bus is None)
// ---------------------------------------------------------------------------

/// Fire-and-forget: send an event to the bus. Silently drops if no subscribers.
pub fn emit(bus: Option<&EventBus>, event: SandboxEvent) {
    if let Some(bus) = bus {
        let _ = bus.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_event_serialization() {
        let event = SandboxEvent {
            event: "sandbox.created".to_string(),
            timestamp: Utc::now(),
            sandbox: "test-sandbox".to_string(),
            labels: HashMap::new(),
            metadata: serde_json::json!({"image": "alpine:3.24"}),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("sandbox.created"));
        assert!(json.contains("test-sandbox"));
    }

    #[test]
    fn test_event_bus_send_recv() {
        let bus = new_event_bus();
        let mut rx = bus.subscribe();
        let event = SandboxEvent {
            event: "sandbox.created".to_string(),
            timestamp: Utc::now(),
            sandbox: "test".to_string(),
            labels: HashMap::new(),
            metadata: serde_json::json!({}),
        };
        bus.send(event.clone()).unwrap();
        let received = rx.try_recv().unwrap();
        assert_eq!(received.event, "sandbox.created");
        assert_eq!(received.sandbox, "test");
    }

    #[test]
    fn test_emit_none_bus() {
        // Should not panic
        emit(
            None,
            SandboxEvent {
                event: "sandbox.created".to_string(),
                timestamp: Utc::now(),
                sandbox: "test".to_string(),
                labels: HashMap::new(),
                metadata: serde_json::json!({}),
            },
        );
    }
}
