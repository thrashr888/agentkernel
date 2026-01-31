//! Policy decision audit logging in OCSF-compatible JSONL format.
//!
//! Appends structured policy decision records to a log file for compliance
//! and forensics. Fields align with OCSF (Open Cybersecurity Schema Framework)
//! authorization audit events where possible.

#![cfg(feature = "enterprise")]

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::cedar::{Action, PolicyEffect};

/// A single policy decision log entry.
///
/// Fields are aligned with OCSF Authorization Event (class_uid: 3003)
/// for compliance reporting compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecisionLog {
    /// ISO 8601 timestamp of the evaluation
    pub timestamp: DateTime<Utc>,
    /// OCSF class_uid for Authorization events
    #[serde(default = "default_class_uid")]
    pub class_uid: u32,
    /// OCSF activity_id: 1=Authorize, 2=Deny
    pub activity_id: u32,
    /// Principal (user) that requested the action
    pub principal: String,
    /// Action that was requested
    pub action: String,
    /// Resource (sandbox) the action targeted
    pub resource: String,
    /// The authorization decision
    pub decision: PolicyEffect,
    /// IDs of policies that matched
    pub matched_policies: Vec<String>,
    /// Evaluation time in microseconds
    pub evaluation_time_us: u64,
    /// Organization ID (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    /// Additional context or reason
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// OCSF severity_id: 0=Unknown, 1=Informational, 2=Low, 3=Medium, 4=High
    #[serde(default = "default_severity")]
    pub severity_id: u32,
    /// OCSF status_id: 1=Success, 2=Failure
    pub status_id: u32,
}

fn default_class_uid() -> u32 {
    3003 // OCSF Authorization class
}

fn default_severity() -> u32 {
    1 // Informational
}

impl PolicyDecisionLog {
    /// Create a new log entry from a policy evaluation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        principal: &str,
        action: Action,
        resource: &str,
        decision: PolicyEffect,
        matched_policies: Vec<String>,
        evaluation_time_us: u64,
        org_id: Option<String>,
        reason: Option<String>,
    ) -> Self {
        let (activity_id, status_id, severity_id) = match decision {
            PolicyEffect::Permit => (1, 1, 1), // Authorize, Success, Informational
            PolicyEffect::Deny => (2, 2, 3),   // Deny, Failure, Medium
        };

        Self {
            timestamp: Utc::now(),
            class_uid: default_class_uid(),
            activity_id,
            principal: principal.to_string(),
            action: action.to_string(),
            resource: resource.to_string(),
            decision,
            matched_policies,
            evaluation_time_us,
            org_id,
            reason,
            severity_id,
            status_id,
        }
    }
}

/// Policy audit logger that writes JSONL to a file.
pub struct PolicyAuditLogger {
    /// Path to the audit log file
    log_path: PathBuf,
}

impl PolicyAuditLogger {
    /// Create a new audit logger writing to the specified file.
    pub fn new(log_path: PathBuf) -> Self {
        Self { log_path }
    }

    /// Create a logger using the default log path.
    ///
    /// Default: `~/.agentkernel/logs/policy-audit.jsonl`
    pub fn default_path() -> Self {
        let log_path = if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home)
                .join(".agentkernel")
                .join("logs")
                .join("policy-audit.jsonl")
        } else {
            PathBuf::from("/tmp/agentkernel/logs/policy-audit.jsonl")
        };
        Self::new(log_path)
    }

    /// Append a decision log entry to the audit log.
    pub fn log_decision(&self, entry: &PolicyDecisionLog) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.log_path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create audit log directory")?;
        }

        let json = serde_json::to_string(entry).context("Failed to serialize audit log entry")?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .context("Failed to open audit log file")?;

        writeln!(file, "{}", json).context("Failed to write audit log entry")?;

        Ok(())
    }

    /// Read all decision logs from the audit file.
    pub fn read_all(&self) -> Result<Vec<PolicyDecisionLog>> {
        if !self.log_path.exists() {
            return Ok(Vec::new());
        }

        let content =
            std::fs::read_to_string(&self.log_path).context("Failed to read audit log")?;

        let mut entries = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<PolicyDecisionLog>(line) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    eprintln!("[enterprise] Skipping malformed audit log entry: {}", e);
                }
            }
        }

        Ok(entries)
    }

    /// Read the last N decision logs.
    pub fn read_last(&self, n: usize) -> Result<Vec<PolicyDecisionLog>> {
        let all = self.read_all()?;
        let start = all.len().saturating_sub(n);
        Ok(all[start..].to_vec())
    }

    /// Get the log file path.
    pub fn path(&self) -> &Path {
        &self.log_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_log_and_read() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("audit.jsonl");
        let logger = PolicyAuditLogger::new(log_path);

        let entry = PolicyDecisionLog::new(
            "alice@acme.com",
            Action::Run,
            "my-sandbox",
            PolicyEffect::Permit,
            vec!["policy0".to_string()],
            150,
            Some("acme-corp".to_string()),
            None,
        );

        logger.log_decision(&entry).unwrap();

        let entries = logger.read_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].principal, "alice@acme.com");
        assert_eq!(entries[0].action, "Run");
        assert_eq!(entries[0].resource, "my-sandbox");
        assert_eq!(entries[0].decision, PolicyEffect::Permit);
        assert_eq!(entries[0].activity_id, 1);
        assert_eq!(entries[0].status_id, 1);
    }

    #[test]
    fn test_multiple_entries() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("audit.jsonl");
        let logger = PolicyAuditLogger::new(log_path);

        for i in 0..5 {
            let entry = PolicyDecisionLog::new(
                &format!("user-{}", i),
                Action::Create,
                "sandbox",
                if i % 2 == 0 {
                    PolicyEffect::Permit
                } else {
                    PolicyEffect::Deny
                },
                vec![],
                100,
                None,
                None,
            );
            logger.log_decision(&entry).unwrap();
        }

        let entries = logger.read_all().unwrap();
        assert_eq!(entries.len(), 5);

        let last_two = logger.read_last(2).unwrap();
        assert_eq!(last_two.len(), 2);
        assert_eq!(last_two[0].principal, "user-3");
        assert_eq!(last_two[1].principal, "user-4");
    }

    #[test]
    fn test_empty_log() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("nonexistent.jsonl");
        let logger = PolicyAuditLogger::new(log_path);

        let entries = logger.read_all().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_deny_entry_ocsf_fields() {
        let entry = PolicyDecisionLog::new(
            "bob@evil.com",
            Action::Network,
            "restricted-sandbox",
            PolicyEffect::Deny,
            vec!["forbid-policy-1".to_string()],
            50,
            Some("acme-corp".to_string()),
            Some("MFA not verified".to_string()),
        );

        assert_eq!(entry.class_uid, 3003);
        assert_eq!(entry.activity_id, 2); // Deny
        assert_eq!(entry.status_id, 2); // Failure
        assert_eq!(entry.severity_id, 3); // Medium
        assert_eq!(entry.reason, Some("MFA not verified".to_string()));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let entry = PolicyDecisionLog::new(
            "alice@acme.com",
            Action::Exec,
            "sandbox-1",
            PolicyEffect::Permit,
            vec!["p1".to_string(), "p2".to_string()],
            200,
            None,
            None,
        );

        let json = serde_json::to_string(&entry).unwrap();
        let restored: PolicyDecisionLog = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.principal, "alice@acme.com");
        assert_eq!(restored.action, "Exec");
        assert_eq!(restored.decision, PolicyEffect::Permit);
        assert_eq!(restored.matched_policies.len(), 2);
    }
}
