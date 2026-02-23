//! Audit logging for agentkernel operations.
//!
//! Logs all sandbox operations to a JSONL file for security auditing.
//! Default location: ~/.agentkernel/audit.jsonl

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

/// Audit event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditEvent {
    /// Sandbox created
    SandboxCreated {
        name: String,
        image: String,
        backend: String,
        #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
        labels: std::collections::HashMap<String, String>,
    },
    /// Sandbox started
    SandboxStarted {
        name: String,
        profile: Option<String>,
    },
    /// Sandbox stopped
    SandboxStopped { name: String },
    /// Sandbox removed
    SandboxRemoved { name: String },
    /// Command executed
    CommandExecuted {
        sandbox: String,
        command: Vec<String>,
        exit_code: Option<i32>,
    },
    /// File written to sandbox
    FileWritten { sandbox: String, path: String },
    /// File read from sandbox
    FileRead { sandbox: String, path: String },
    /// Session attached
    SessionAttached { sandbox: String },
    /// Policy violation (for future use)
    PolicyViolation {
        sandbox: String,
        policy: String,
        details: String,
    },
    /// SSH session started
    SshConnected {
        sandbox: String,
        host_port: u16,
        ssh_user: String,
    },
    /// SSH session ended
    SshDisconnected {
        sandbox: String,
        duration_secs: u64,
        recording: Option<String>,
    },
    /// Sandbox error (e.g. init script failure)
    SandboxError { name: String, error: String },
    /// Schedule triggered (manually or by cron)
    ScheduleTriggered {
        schedule_id: String,
        schedule_name: String,
        method: String,
    },
}

/// A logged audit entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Timestamp of the event
    pub timestamp: DateTime<Utc>,
    /// Process ID
    pub pid: u32,
    /// Username (from environment)
    pub user: Option<String>,
    /// The event
    #[serde(flatten)]
    pub event: AuditEvent,
}

impl AuditEntry {
    /// Create a new audit entry for an event
    pub fn new(event: AuditEvent) -> Self {
        Self {
            timestamp: Utc::now(),
            pid: std::process::id(),
            user: std::env::var("USER").ok(),
            event,
        }
    }
}

/// Get the default audit log path
pub fn default_audit_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".agentkernel")
        .join("audit.jsonl")
}

/// Audit logger
pub struct AuditLog {
    path: PathBuf,
    enabled: bool,
}

impl AuditLog {
    /// Create a new audit logger
    pub fn new() -> Self {
        let enabled = std::env::var("AGENTKERNEL_AUDIT")
            .map(|v| v != "0" && v.to_lowercase() != "false")
            .unwrap_or(true); // Enabled by default

        Self {
            path: default_audit_path(),
            enabled,
        }
    }

    /// Create with a custom path
    #[allow(dead_code)]
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            path,
            enabled: true,
        }
    }

    /// Log an audit event
    pub fn log(&self, event: AuditEvent) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let entry = AuditEntry::new(event);
        let line = serde_json::to_string(&entry)?;

        // Ensure directory exists
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Append to log file
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        writeln!(file, "{}", line)?;
        Ok(())
    }

    /// Read all audit entries
    pub fn read_all(&self) -> Result<Vec<AuditEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(entry) => entries.push(entry),
                Err(_) => {
                    // Legacy ssh_connected entries have duplicate "user" keys
                    // (AuditEntry.user + SshConnected.user before the rename to ssh_user).
                    // Fix up the second occurrence and retry.
                    if line.contains("\"ssh_connected\"")
                        && let Some(second) = line.rfind("\"user\":")
                    {
                        let mut fixed = line.clone();
                        fixed.replace_range(second..second + 7, "\"ssh_user\":");
                        if let Ok(entry) = serde_json::from_str(&fixed) {
                            entries.push(entry);
                            continue;
                        }
                    }
                    eprintln!("Warning: skipping malformed audit entry");
                }
            }
        }

        Ok(entries)
    }

    /// Read entries filtered by sandbox name
    pub fn read_by_sandbox(&self, sandbox: &str) -> Result<Vec<AuditEntry>> {
        let entries = self.read_all()?;
        Ok(entries
            .into_iter()
            .filter(|e| match &e.event {
                AuditEvent::SandboxCreated { name, .. } => name == sandbox,
                AuditEvent::SandboxStarted { name, .. } => name == sandbox,
                AuditEvent::SandboxStopped { name } => name == sandbox,
                AuditEvent::SandboxRemoved { name } => name == sandbox,
                AuditEvent::CommandExecuted { sandbox: s, .. } => s == sandbox,
                AuditEvent::FileWritten { sandbox: s, .. } => s == sandbox,
                AuditEvent::FileRead { sandbox: s, .. } => s == sandbox,
                AuditEvent::SessionAttached { sandbox: s } => s == sandbox,
                AuditEvent::PolicyViolation { sandbox: s, .. } => s == sandbox,
                AuditEvent::SshConnected { sandbox: s, .. } => s == sandbox,
                AuditEvent::SshDisconnected { sandbox: s, .. } => s == sandbox,
                AuditEvent::SandboxError { name, .. } => name == sandbox,
                AuditEvent::ScheduleTriggered { .. } => false,
            })
            .collect())
    }

    /// Read the last N entries
    pub fn read_last(&self, n: usize) -> Result<Vec<AuditEntry>> {
        let entries = self.read_all()?;
        let start = entries.len().saturating_sub(n);
        Ok(entries[start..].to_vec())
    }

    /// Get the log path
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Global audit logger (lazy initialized)
pub fn audit() -> &'static AuditLog {
    use std::sync::OnceLock;
    static AUDIT: OnceLock<AuditLog> = OnceLock::new();
    AUDIT.get_or_init(AuditLog::new)
}

/// Convenience function to log an event
pub fn log_event(event: AuditEvent) {
    if let Err(e) = audit().log(event) {
        eprintln!("Warning: failed to write audit log: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_audit_entry_serialization() {
        let entry = AuditEntry::new(AuditEvent::SandboxCreated {
            name: "test".to_string(),
            image: "alpine:3.20".to_string(),
            backend: "docker".to_string(),
            labels: Default::default(),
        });

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"type\":\"sandbox_created\""));
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"timestamp\""));
    }

    #[test]
    fn test_audit_log_write_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = AuditLog::with_path(path);

        // Write events
        log.log(AuditEvent::SandboxCreated {
            name: "test1".to_string(),
            image: "alpine".to_string(),
            backend: "docker".to_string(),
            labels: Default::default(),
        })
        .unwrap();

        log.log(AuditEvent::CommandExecuted {
            sandbox: "test1".to_string(),
            command: vec!["echo".to_string(), "hello".to_string()],
            exit_code: Some(0),
        })
        .unwrap();

        // Read back
        let entries = log.read_all().unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_audit_log_filter_by_sandbox() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = AuditLog::with_path(path);

        log.log(AuditEvent::SandboxCreated {
            name: "test1".to_string(),
            image: "alpine".to_string(),
            backend: "docker".to_string(),
            labels: Default::default(),
        })
        .unwrap();

        log.log(AuditEvent::SandboxCreated {
            name: "test2".to_string(),
            image: "alpine".to_string(),
            backend: "docker".to_string(),
            labels: Default::default(),
        })
        .unwrap();

        let filtered = log.read_by_sandbox("test1").unwrap();
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_ssh_connected_event() {
        let event = AuditEvent::SshConnected {
            sandbox: "test-box".to_string(),
            host_port: 2222,
            ssh_user: "sandbox".to_string(),
        };
        let entry = AuditEntry::new(event);
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("ssh_connected"));
        assert!(json.contains("test-box"));
        assert!(json.contains("2222"));
        assert!(json.contains("ssh_user"));

        // Roundtrip: verify deserialization succeeds (no duplicate field error)
        let parsed: AuditEntry = serde_json::from_str(&json).unwrap();
        if let AuditEvent::SshConnected { ssh_user, .. } = &parsed.event {
            assert_eq!(ssh_user, "sandbox");
        } else {
            panic!("Expected SshConnected event after roundtrip");
        }
        // AuditEntry.user (OS user) must not collide with ssh_user
        assert!(parsed.user.is_some());
    }

    #[test]
    fn test_ssh_disconnected_event() {
        let event = AuditEvent::SshDisconnected {
            sandbox: "test-box".to_string(),
            duration_secs: 120,
            recording: Some("/tmp/session.cast".to_string()),
        };
        let entry = AuditEntry::new(event);
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("ssh_disconnected"));
        assert!(json.contains("120"));
        assert!(json.contains("session.cast"));
    }

    #[test]
    fn test_ssh_disconnected_without_recording() {
        let event = AuditEvent::SshDisconnected {
            sandbox: "test-box".to_string(),
            duration_secs: 60,
            recording: None,
        };
        let entry = AuditEntry::new(event);
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("ssh_disconnected"));
        assert!(!json.contains("session.cast"));
    }
}
