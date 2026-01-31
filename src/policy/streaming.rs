//! Audit log streaming for enterprise policy decisions.
//!
//! Supports streaming audit events to external destinations:
//! - HTTP webhooks (POST JSON arrays)
//! - File (append JSONL)
//! - Stdout (for debugging and piping)
//!
//! Events include OCSF-compatible metadata fields for compliance integration
//! with SIEM systems (Splunk, Datadog, etc.).

#[cfg(feature = "enterprise")]
use anyhow::{Context, Result};
#[cfg(feature = "enterprise")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "enterprise")]
use std::path::PathBuf;
#[cfg(feature = "enterprise")]
use std::sync::Arc;
#[cfg(feature = "enterprise")]
use tokio::sync::Mutex;

/// Destination for streaming audit events.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamDestination {
    /// POST JSON array to an HTTP webhook URL
    HttpWebhook {
        /// Webhook URL
        url: String,
        /// Optional authorization header value (e.g., "Bearer token123")
        #[serde(default)]
        authorization: Option<String>,
        /// Optional custom headers
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
    },
    /// Append JSONL to a local file
    File {
        /// Path to the output file
        path: String,
    },
    /// Write to stdout (for debugging and piping)
    Stdout,
}

/// Configuration for the audit event streamer.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditStreamConfig {
    /// Where to send audit events
    pub destination: StreamDestination,
    /// Number of events to batch before sending (default: 10)
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// Flush interval in seconds (default: 30)
    #[serde(default = "default_flush_interval")]
    pub flush_interval_secs: u64,
    /// Maximum retry attempts for HTTP webhook delivery (default: 3)
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Include OCSF metadata in events (default: true)
    #[serde(default = "default_ocsf_enabled")]
    pub ocsf_enabled: bool,
}

#[cfg(feature = "enterprise")]
fn default_batch_size() -> usize {
    10
}

#[cfg(feature = "enterprise")]
fn default_flush_interval() -> u64 {
    30
}

#[cfg(feature = "enterprise")]
fn default_max_retries() -> u32 {
    3
}

#[cfg(feature = "enterprise")]
fn default_ocsf_enabled() -> bool {
    true
}

#[cfg(feature = "enterprise")]
impl Default for AuditStreamConfig {
    fn default() -> Self {
        Self {
            destination: StreamDestination::Stdout,
            batch_size: default_batch_size(),
            flush_interval_secs: default_flush_interval(),
            max_retries: default_max_retries(),
            ocsf_enabled: default_ocsf_enabled(),
        }
    }
}

/// OCSF (Open Cybersecurity Schema Framework) compatible audit event.
///
/// Maps to OCSF Base Event class (class_uid: 0) with agentkernel-specific
/// extensions for sandbox operations and policy decisions.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Event timestamp (ISO 8601)
    pub time: String,
    /// OCSF class UID (3001 = API Activity for sandbox ops)
    #[serde(default = "default_class_uid")]
    pub class_uid: u32,
    /// OCSF category UID (3 = Audit Activity)
    #[serde(default = "default_category_uid")]
    pub category_uid: u32,
    /// OCSF severity (1=Info, 2=Low, 3=Medium, 4=High, 5=Critical)
    #[serde(default = "default_severity_id")]
    pub severity_id: u32,
    /// Event type (e.g., "policy_decision", "sandbox_operation", "auth_event")
    pub type_name: String,
    /// Unique event ID
    pub uid: String,
    /// The action that was evaluated
    pub action: String,
    /// The outcome of the action
    pub outcome: EventOutcome,
    /// Actor information (who performed the action)
    #[serde(default)]
    pub actor: Option<ActorInfo>,
    /// Resource information (what was acted upon)
    #[serde(default)]
    pub resource: Option<ResourceInfo>,
    /// Policy information (which policy applied)
    #[serde(default)]
    pub policy: Option<PolicyInfo>,
    /// Additional metadata
    #[serde(default)]
    pub metadata: EventMetadata,
}

#[cfg(feature = "enterprise")]
fn default_class_uid() -> u32 {
    3001
}

#[cfg(feature = "enterprise")]
fn default_category_uid() -> u32 {
    3
}

#[cfg(feature = "enterprise")]
fn default_severity_id() -> u32 {
    1
}

/// Event outcome (permit/deny/error).
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventOutcome {
    /// Action was permitted
    Permit,
    /// Action was denied
    Deny,
    /// Action evaluation failed
    Error,
    /// Informational (no decision needed)
    Info,
}

/// Information about who performed an action.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorInfo {
    /// User identifier (from JWT sub claim)
    #[serde(default)]
    pub user_id: Option<String>,
    /// User email
    #[serde(default)]
    pub email: Option<String>,
    /// Organization ID
    #[serde(default)]
    pub org_id: Option<String>,
    /// IP address (if available)
    #[serde(default)]
    pub ip_address: Option<String>,
}

/// Information about the resource being acted upon.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceInfo {
    /// Resource type (e.g., "sandbox", "file", "network")
    pub resource_type: String,
    /// Resource identifier (e.g., sandbox name)
    #[serde(default)]
    pub resource_id: Option<String>,
    /// Additional resource attributes
    #[serde(default)]
    pub attributes: std::collections::HashMap<String, serde_json::Value>,
}

/// Policy information for decision events.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyInfo {
    /// Policy ID that was evaluated
    pub policy_id: String,
    /// Policy name
    #[serde(default)]
    pub policy_name: Option<String>,
    /// Policy version
    #[serde(default)]
    pub policy_version: Option<u64>,
}

/// Event metadata following OCSF conventions.
#[cfg(feature = "enterprise")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMetadata {
    /// Product name
    #[serde(default = "default_product_name")]
    pub product_name: String,
    /// Product vendor
    #[serde(default = "default_vendor_name")]
    pub vendor_name: String,
    /// Product version
    #[serde(default = "default_product_version")]
    pub product_version: String,
    /// Hostname where the event occurred
    #[serde(default)]
    pub hostname: Option<String>,
}

#[cfg(feature = "enterprise")]
fn default_product_name() -> String {
    "agentkernel".to_string()
}

#[cfg(feature = "enterprise")]
fn default_vendor_name() -> String {
    "agentkernel".to_string()
}

#[cfg(feature = "enterprise")]
fn default_product_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(feature = "enterprise")]
impl Default for EventMetadata {
    fn default() -> Self {
        Self {
            product_name: default_product_name(),
            vendor_name: default_vendor_name(),
            product_version: default_product_version(),
            hostname: hostname(),
        }
    }
}

/// Get the system hostname.
#[cfg(feature = "enterprise")]
fn hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
}

/// Audit event streamer with batching and background flush.
#[cfg(feature = "enterprise")]
pub struct AuditStreamer {
    config: AuditStreamConfig,
    buffer: Arc<Mutex<Vec<AuditEvent>>>,
    client: reqwest::Client,
}

#[cfg(feature = "enterprise")]
impl AuditStreamer {
    /// Create a new audit streamer with the given configuration.
    pub fn new(config: AuditStreamConfig) -> Self {
        Self {
            config,
            buffer: Arc::new(Mutex::new(Vec::new())),
            client: reqwest::Client::new(),
        }
    }

    /// Queue an event for streaming. Events are batched and flushed
    /// when the batch size is reached or the flush interval expires.
    pub async fn queue_event(&self, event: AuditEvent) -> Result<()> {
        let should_flush = {
            let mut buffer = self.buffer.lock().await;
            buffer.push(event);
            buffer.len() >= self.config.batch_size
        };

        if should_flush {
            self.flush().await?;
        }

        Ok(())
    }

    /// Stream a batch of events immediately (bypass buffering).
    pub async fn stream_events(&self, events: Vec<AuditEvent>) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        match &self.config.destination {
            StreamDestination::HttpWebhook {
                url,
                authorization,
                headers,
            } => {
                self.send_http_webhook(url, authorization, headers, &events)
                    .await
            }
            StreamDestination::File { path } => self.append_to_file(path, &events),
            StreamDestination::Stdout => self.write_to_stdout(&events),
        }
    }

    /// Flush all buffered events to the destination.
    pub async fn flush(&self) -> Result<()> {
        let events = {
            let mut buffer = self.buffer.lock().await;
            std::mem::take(&mut *buffer)
        };

        if !events.is_empty() {
            self.stream_events(events).await?;
        }

        Ok(())
    }

    /// Start a background flush task that periodically flushes buffered events.
    ///
    /// Returns a JoinHandle for the background task. The task runs until
    /// the returned handle is dropped or aborted.
    pub fn start_background_flush(&self) -> tokio::task::JoinHandle<()> {
        let buffer = Arc::clone(&self.buffer);
        let config = self.config.clone();
        let client = self.client.clone();
        let interval = std::time::Duration::from_secs(config.flush_interval_secs);

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // Skip the first immediate tick

            loop {
                ticker.tick().await;

                let events = {
                    let mut buf = buffer.lock().await;
                    std::mem::take(&mut *buf)
                };

                if events.is_empty() {
                    continue;
                }

                // Create a temporary streamer for flushing
                let streamer = AuditStreamer {
                    config: config.clone(),
                    buffer: Arc::new(Mutex::new(Vec::new())),
                    client: client.clone(),
                };

                if let Err(e) = streamer.stream_events(events).await {
                    eprintln!("Warning: audit stream flush failed: {}", e);
                }
            }
        })
    }

    /// Send events to an HTTP webhook endpoint.
    async fn send_http_webhook(
        &self,
        url: &str,
        authorization: &Option<String>,
        headers: &std::collections::HashMap<String, String>,
        events: &[AuditEvent],
    ) -> Result<()> {
        let mut retries = 0;

        loop {
            let mut request = self
                .client
                .post(url)
                .header("Content-Type", "application/json")
                .json(events);

            if let Some(auth) = authorization {
                request = request.header("Authorization", auth);
            }

            for (key, value) in headers {
                request = request.header(key, value);
            }

            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    return Ok(());
                }
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();

                    if retries < self.config.max_retries
                        && (status.is_server_error() || status.as_u16() == 429)
                    {
                        retries += 1;
                        let backoff = std::time::Duration::from_secs(2u64.pow(retries));
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    anyhow::bail!(
                        "Webhook delivery failed after {} retries ({}): {}",
                        retries,
                        status,
                        body
                    );
                }
                Err(e) => {
                    if retries < self.config.max_retries {
                        retries += 1;
                        let backoff = std::time::Duration::from_secs(2u64.pow(retries));
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    return Err(e).context(format!(
                        "Webhook delivery failed after {} retries",
                        retries
                    ));
                }
            }
        }
    }

    /// Append events as JSONL to a file.
    fn append_to_file(&self, path: &str, events: &[AuditEvent]) -> Result<()> {
        use std::io::Write;

        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .context("Failed to create audit stream output directory")?;
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("Failed to open audit stream file: {}", path.display()))?;

        for event in events {
            let line = serde_json::to_string(event)
                .context("Failed to serialize audit event")?;
            writeln!(file, "{}", line)?;
        }

        Ok(())
    }

    /// Write events as JSONL to stdout.
    fn write_to_stdout(&self, events: &[AuditEvent]) -> Result<()> {
        for event in events {
            let line = serde_json::to_string(event)
                .context("Failed to serialize audit event")?;
            println!("{}", line);
        }
        Ok(())
    }

    /// Get the current number of buffered events.
    pub async fn buffered_count(&self) -> usize {
        self.buffer.lock().await.len()
    }
}

/// Create a new AuditEvent with sensible defaults.
#[cfg(feature = "enterprise")]
pub fn new_audit_event(type_name: &str, action: &str, outcome: EventOutcome) -> AuditEvent {
    AuditEvent {
        time: chrono::Utc::now().to_rfc3339(),
        class_uid: default_class_uid(),
        category_uid: default_category_uid(),
        severity_id: match &outcome {
            EventOutcome::Permit => 1,
            EventOutcome::Info => 1,
            EventOutcome::Deny => 3,
            EventOutcome::Error => 4,
        },
        type_name: type_name.to_string(),
        uid: uuid::Uuid::new_v4().to_string(),
        action: action.to_string(),
        outcome,
        actor: None,
        resource: None,
        policy: None,
        metadata: EventMetadata::default(),
    }
}

#[cfg(all(test, feature = "enterprise"))]
mod tests {
    use super::*;

    fn sample_event() -> AuditEvent {
        new_audit_event("policy_decision", "Run", EventOutcome::Permit)
    }

    fn sample_deny_event() -> AuditEvent {
        let mut event = new_audit_event("policy_decision", "Network", EventOutcome::Deny);
        event.actor = Some(ActorInfo {
            user_id: Some("user-123".to_string()),
            email: Some("user@example.com".to_string()),
            org_id: Some("acme-corp".to_string()),
            ip_address: None,
        });
        event.resource = Some(ResourceInfo {
            resource_type: "sandbox".to_string(),
            resource_id: Some("dev-sandbox".to_string()),
            attributes: std::collections::HashMap::new(),
        });
        event.policy = Some(PolicyInfo {
            policy_id: "no-network-healthcare".to_string(),
            policy_name: Some("Healthcare Network Restriction".to_string()),
            policy_version: Some(3),
        });
        event
    }

    #[test]
    fn test_audit_event_serialization() {
        let event = sample_event();
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type_name\":\"policy_decision\""));
        assert!(json.contains("\"action\":\"Run\""));
        assert!(json.contains("\"product_name\":\"agentkernel\""));
    }

    #[test]
    fn test_audit_event_with_full_context() {
        let event = sample_deny_event();
        let json = serde_json::to_string_pretty(&event).unwrap();

        let deserialized: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.action, "Network");
        assert!(matches!(deserialized.outcome, EventOutcome::Deny));
        assert_eq!(
            deserialized.actor.unwrap().user_id,
            Some("user-123".to_string())
        );
        assert_eq!(
            deserialized.resource.unwrap().resource_type,
            "sandbox"
        );
        assert_eq!(
            deserialized.policy.unwrap().policy_id,
            "no-network-healthcare"
        );
    }

    #[test]
    fn test_stream_destination_deserialization() {
        let json = r#"{"type": "http_webhook", "url": "https://hooks.example.com/audit"}"#;
        let dest: StreamDestination = serde_json::from_str(json).unwrap();
        match dest {
            StreamDestination::HttpWebhook { url, .. } => {
                assert_eq!(url, "https://hooks.example.com/audit");
            }
            _ => panic!("Expected HttpWebhook"),
        }

        let json = r#"{"type": "file", "path": "/var/log/agentkernel/audit.jsonl"}"#;
        let dest: StreamDestination = serde_json::from_str(json).unwrap();
        match dest {
            StreamDestination::File { path } => {
                assert_eq!(path, "/var/log/agentkernel/audit.jsonl");
            }
            _ => panic!("Expected File"),
        }

        let json = r#"{"type": "stdout"}"#;
        let dest: StreamDestination = serde_json::from_str(json).unwrap();
        assert!(matches!(dest, StreamDestination::Stdout));
    }

    #[test]
    fn test_audit_stream_config_defaults() {
        let config = AuditStreamConfig::default();
        assert_eq!(config.batch_size, 10);
        assert_eq!(config.flush_interval_secs, 30);
        assert_eq!(config.max_retries, 3);
        assert!(config.ocsf_enabled);
    }

    #[test]
    fn test_audit_stream_config_deserialization() {
        let json = r#"{
            "destination": {"type": "http_webhook", "url": "https://example.com/webhook", "authorization": "Bearer tok123"},
            "batch_size": 50,
            "flush_interval_secs": 60,
            "max_retries": 5,
            "ocsf_enabled": true
        }"#;

        let config: AuditStreamConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.batch_size, 50);
        assert_eq!(config.flush_interval_secs, 60);
        assert_eq!(config.max_retries, 5);
    }

    #[test]
    fn test_event_metadata_defaults() {
        let meta = EventMetadata::default();
        assert_eq!(meta.product_name, "agentkernel");
        assert_eq!(meta.vendor_name, "agentkernel");
        assert!(!meta.product_version.is_empty());
    }

    #[test]
    fn test_severity_mapping() {
        let permit = new_audit_event("test", "action", EventOutcome::Permit);
        assert_eq!(permit.severity_id, 1);

        let deny = new_audit_event("test", "action", EventOutcome::Deny);
        assert_eq!(deny.severity_id, 3);

        let error = new_audit_event("test", "action", EventOutcome::Error);
        assert_eq!(error.severity_id, 4);
    }

    #[tokio::test]
    async fn test_streamer_file_output() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("audit.jsonl");

        let config = AuditStreamConfig {
            destination: StreamDestination::File {
                path: file_path.to_string_lossy().to_string(),
            },
            batch_size: 10,
            flush_interval_secs: 30,
            max_retries: 3,
            ocsf_enabled: true,
        };

        let streamer = AuditStreamer::new(config);

        // Stream some events directly
        let events = vec![sample_event(), sample_deny_event()];
        streamer.stream_events(events).await.unwrap();

        // Verify file contents
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        // Verify each line is valid JSON
        for line in &lines {
            let _event: AuditEvent = serde_json::from_str(line).unwrap();
        }
    }

    #[tokio::test]
    async fn test_streamer_batching() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("batch-audit.jsonl");

        let config = AuditStreamConfig {
            destination: StreamDestination::File {
                path: file_path.to_string_lossy().to_string(),
            },
            batch_size: 3,
            flush_interval_secs: 300,
            max_retries: 1,
            ocsf_enabled: true,
        };

        let streamer = AuditStreamer::new(config);

        // Queue events one at a time
        streamer.queue_event(sample_event()).await.unwrap();
        assert_eq!(streamer.buffered_count().await, 1);

        streamer.queue_event(sample_event()).await.unwrap();
        assert_eq!(streamer.buffered_count().await, 2);

        // Third event should trigger flush (batch_size = 3)
        streamer.queue_event(sample_event()).await.unwrap();
        assert_eq!(streamer.buffered_count().await, 0);

        // Verify file has 3 events
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content.lines().count(), 3);
    }

    #[tokio::test]
    async fn test_streamer_manual_flush() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("flush-audit.jsonl");

        let config = AuditStreamConfig {
            destination: StreamDestination::File {
                path: file_path.to_string_lossy().to_string(),
            },
            batch_size: 100, // High threshold so auto-flush doesn't trigger
            flush_interval_secs: 300,
            max_retries: 1,
            ocsf_enabled: true,
        };

        let streamer = AuditStreamer::new(config);

        streamer.queue_event(sample_event()).await.unwrap();
        streamer.queue_event(sample_event()).await.unwrap();
        assert_eq!(streamer.buffered_count().await, 2);

        // File shouldn't exist yet (no flush)
        assert!(!file_path.exists());

        // Manual flush
        streamer.flush().await.unwrap();
        assert_eq!(streamer.buffered_count().await, 0);

        // Now file should have 2 events
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content.lines().count(), 2);
    }
}
