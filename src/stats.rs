//! Local analytics aggregated from the audit log.

use anyhow::Result;
use chrono::{Duration, Utc};
use serde::Serialize;
use std::collections::HashMap;

use crate::audit::{AuditEntry, AuditEvent, AuditLog};

/// Aggregated statistics from the audit log
#[derive(Debug, Serialize)]
pub struct Stats {
    pub total_executions: usize,
    pub executions_last_30d: usize,
    pub sandboxes_created: usize,
    pub sandboxes_removed: usize,
    pub sandboxes_active: usize,
    pub top_images: Vec<(String, usize)>,
    pub top_backends: Vec<(String, usize)>,
    pub first_entry: Option<String>,
    pub last_entry: Option<String>,
}

impl Stats {
    /// Compute stats from audit log entries
    pub fn from_entries(entries: &[AuditEntry]) -> Self {
        let now = Utc::now();
        let thirty_days_ago = now - Duration::days(30);

        let mut total_executions = 0usize;
        let mut executions_last_30d = 0usize;
        let mut sandboxes_created = 0usize;
        let mut sandboxes_removed = 0usize;
        let mut image_counts: HashMap<String, usize> = HashMap::new();
        let mut backend_counts: HashMap<String, usize> = HashMap::new();
        // Track sandbox lifecycle to compute "active" (created but not removed)
        let mut active_sandboxes: HashMap<String, bool> = HashMap::new();

        for entry in entries {
            match &entry.event {
                AuditEvent::CommandExecuted { .. } => {
                    total_executions += 1;
                    if entry.timestamp >= thirty_days_ago {
                        executions_last_30d += 1;
                    }
                }
                AuditEvent::SandboxCreated {
                    name,
                    image,
                    backend,
                    ..
                } => {
                    sandboxes_created += 1;
                    *image_counts.entry(image.clone()).or_default() += 1;
                    *backend_counts.entry(backend.clone()).or_default() += 1;
                    active_sandboxes.insert(name.clone(), true);
                }
                AuditEvent::SandboxRemoved { name } => {
                    sandboxes_removed += 1;
                    active_sandboxes.insert(name.clone(), false);
                }
                _ => {}
            }
        }

        let sandboxes_active = active_sandboxes.values().filter(|&&v| v).count();

        // Sort by count descending, take top 5
        let mut top_images: Vec<(String, usize)> = image_counts.into_iter().collect();
        top_images.sort_by_key(|item| std::cmp::Reverse(item.1));
        top_images.truncate(5);

        let mut top_backends: Vec<(String, usize)> = backend_counts.into_iter().collect();
        top_backends.sort_by_key(|item| std::cmp::Reverse(item.1));
        top_backends.truncate(5);

        let first_entry = entries
            .first()
            .map(|e| e.timestamp.format("%Y-%m-%d").to_string());
        let last_entry = entries
            .last()
            .map(|e| e.timestamp.format("%Y-%m-%d").to_string());

        Stats {
            total_executions,
            executions_last_30d,
            sandboxes_created,
            sandboxes_removed,
            sandboxes_active,
            top_images,
            top_backends,
            first_entry,
            last_entry,
        }
    }

    /// Print stats in human-readable format
    pub fn print(&self) {
        println!(
            "Executions:     {} total (last 30d: {})",
            self.total_executions, self.executions_last_30d
        );
        println!(
            "Sandboxes:      {} created, {} removed, {} active",
            self.sandboxes_created, self.sandboxes_removed, self.sandboxes_active
        );

        if !self.top_images.is_empty() {
            let images_str: Vec<String> = self
                .top_images
                .iter()
                .map(|(img, count)| format!("{} ({})", img, count))
                .collect();
            println!("Top images:     {}", images_str.join(", "));
        }

        if !self.top_backends.is_empty() {
            let backends_str: Vec<String> = self
                .top_backends
                .iter()
                .map(|(b, count)| format!("{} ({})", b, count))
                .collect();
            println!("Top backends:   {}", backends_str.join(", "));
        }

        if let (Some(first), Some(last)) = (&self.first_entry, &self.last_entry) {
            println!("First entry:    {}  Last: {}", first, last);
        }
    }
}

/// Compute stats from the default audit log
pub fn compute_stats() -> Result<Stats> {
    let log = AuditLog::new();
    let entries = log.read_all()?;
    Ok(Stats::from_entries(&entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_entry(event: AuditEvent) -> AuditEntry {
        AuditEntry {
            timestamp: Utc::now(),
            pid: 1,
            user: Some("test".to_string()),
            event,
        }
    }

    #[test]
    fn test_empty_stats() {
        let stats = Stats::from_entries(&[]);
        assert_eq!(stats.total_executions, 0);
        assert_eq!(stats.sandboxes_created, 0);
        assert!(stats.first_entry.is_none());
    }

    #[test]
    fn test_stats_counts() {
        let entries = vec![
            make_entry(AuditEvent::SandboxCreated {
                name: "test1".to_string(),
                image: "alpine:3.24".to_string(),
                backend: "docker".to_string(),
                labels: Default::default(),
            }),
            make_entry(AuditEvent::CommandExecuted {
                sandbox: "test1".to_string(),
                command: vec!["echo".to_string(), "hello".to_string()],
                exit_code: Some(0),
            }),
            make_entry(AuditEvent::CommandExecuted {
                sandbox: "test1".to_string(),
                command: vec!["ls".to_string()],
                exit_code: Some(0),
            }),
            make_entry(AuditEvent::SandboxRemoved {
                name: "test1".to_string(),
            }),
        ];

        let stats = Stats::from_entries(&entries);
        assert_eq!(stats.total_executions, 2);
        assert_eq!(stats.executions_last_30d, 2);
        assert_eq!(stats.sandboxes_created, 1);
        assert_eq!(stats.sandboxes_removed, 1);
        assert_eq!(stats.sandboxes_active, 0);
        assert_eq!(stats.top_images, vec![("alpine:3.24".to_string(), 1)]);
        assert_eq!(stats.top_backends, vec![("docker".to_string(), 1)]);
    }

    #[test]
    fn test_active_sandboxes() {
        let entries = vec![
            make_entry(AuditEvent::SandboxCreated {
                name: "a".to_string(),
                image: "alpine".to_string(),
                backend: "docker".to_string(),
                labels: Default::default(),
            }),
            make_entry(AuditEvent::SandboxCreated {
                name: "b".to_string(),
                image: "alpine".to_string(),
                backend: "docker".to_string(),
                labels: Default::default(),
            }),
            make_entry(AuditEvent::SandboxRemoved {
                name: "a".to_string(),
            }),
        ];

        let stats = Stats::from_entries(&entries);
        assert_eq!(stats.sandboxes_active, 1);
    }
}
