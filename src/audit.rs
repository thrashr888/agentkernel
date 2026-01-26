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
                Err(e) => eprintln!("Warning: skipping malformed audit entry: {}", e),
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
        })
        .unwrap();

        log.log(AuditEvent::SandboxCreated {
            name: "test2".to_string(),
            image: "alpine".to_string(),
            backend: "docker".to_string(),
        })
        .unwrap();

        let filtered = log.read_by_sandbox("test1").unwrap();
        assert_eq!(filtered.len(), 1);
    }
}
