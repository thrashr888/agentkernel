//! Snapshot and restore for sandboxes.
//!
//! Save sandbox state (Docker: `docker commit` + metadata) and restore later.
//! Snapshots are stored in `~/.local/share/agentkernel/snapshots/`.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Snapshot metadata persisted alongside the committed image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    /// Snapshot name (user-facing)
    pub name: String,
    /// Original sandbox name
    pub sandbox: String,
    /// Docker image tag for the committed snapshot
    pub image_tag: String,
    /// Backend used by the original sandbox
    pub backend: String,
    /// Original base image
    pub base_image: String,
    /// vCPU count
    pub vcpus: u32,
    /// Memory in MB
    pub memory_mb: u64,
    /// When the snapshot was taken (RFC3339)
    pub created_at: String,
}

/// Directory where snapshot metadata lives.
fn snapshots_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/agentkernel/snapshots")
}

/// List all snapshots.
pub fn list() -> Result<Vec<SnapshotMeta>> {
    let dir = snapshots_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut snapshots = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            if let Ok(meta) = serde_json::from_str::<SnapshotMeta>(&content) {
                snapshots.push(meta);
            }
        }
    }
    snapshots.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(snapshots)
}

/// Take a snapshot of a running or stopped Docker container.
pub fn take(
    sandbox_name: &str,
    snapshot_name: &str,
    state: &SnapshotInput,
) -> Result<SnapshotMeta> {
    let image_tag = format!("agentkernel-snap:{}", snapshot_name);

    // Docker commit the container
    let output = std::process::Command::new("docker")
        .args(["commit", sandbox_name, &image_tag])
        .output()
        .context("Failed to run docker commit")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("docker commit failed: {}", stderr.trim());
    }

    let meta = SnapshotMeta {
        name: snapshot_name.to_string(),
        sandbox: sandbox_name.to_string(),
        image_tag,
        backend: state.backend.clone(),
        base_image: state.image.clone(),
        vcpus: state.vcpus,
        memory_mb: state.memory_mb,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    // Save metadata
    let dir = snapshots_dir();
    std::fs::create_dir_all(&dir)?;
    let meta_path = dir.join(format!("{}.json", snapshot_name));
    let content = serde_json::to_string_pretty(&meta)?;
    std::fs::write(&meta_path, content)?;

    Ok(meta)
}

/// Input needed from the sandbox state to create a snapshot.
pub struct SnapshotInput {
    pub image: String,
    pub backend: String,
    pub vcpus: u32,
    pub memory_mb: u64,
}

/// Get a snapshot by name.
pub fn get(name: &str) -> Result<Option<SnapshotMeta>> {
    let path = snapshots_dir().join(format!("{}.json", name));
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let meta: SnapshotMeta = serde_json::from_str(&content)?;
    Ok(Some(meta))
}

/// Delete a snapshot (removes metadata and Docker image).
pub fn delete(name: &str) -> Result<()> {
    let meta = get(name)?.ok_or_else(|| anyhow::anyhow!("Snapshot '{}' not found", name))?;

    // Remove Docker image
    let _ = std::process::Command::new("docker")
        .args(["rmi", &meta.image_tag])
        .output();

    // Remove metadata file
    let path = snapshots_dir().join(format!("{}.json", name));
    if path.exists() {
        std::fs::remove_file(&path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshots_dir() {
        let dir = snapshots_dir();
        assert!(dir.to_string_lossy().contains("agentkernel/snapshots"));
    }

    #[test]
    fn test_list_empty() {
        // Should not panic even if directory doesn't exist
        let result = list();
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_missing() {
        let result = get("nonexistent-snapshot-12345").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_missing() {
        let result = delete("nonexistent-snapshot-12345");
        assert!(result.is_err());
    }

    #[test]
    fn test_snapshot_meta_roundtrip() {
        let meta = SnapshotMeta {
            name: "test-snap".to_string(),
            sandbox: "my-sandbox".to_string(),
            image_tag: "agentkernel-snap:test-snap".to_string(),
            backend: "docker".to_string(),
            base_image: "alpine:3.20".to_string(),
            vcpus: 2,
            memory_mb: 512,
            created_at: "2026-02-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&meta).unwrap();
        let parsed: SnapshotMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test-snap");
        assert_eq!(parsed.sandbox, "my-sandbox");
        assert_eq!(parsed.vcpus, 2);
    }

    #[test]
    fn test_snapshot_meta_serialize_fields() {
        let meta = SnapshotMeta {
            name: "snap1".to_string(),
            sandbox: "sb1".to_string(),
            image_tag: "agentkernel-snap:snap1".to_string(),
            backend: "docker".to_string(),
            base_image: "python:3.12".to_string(),
            vcpus: 1,
            memory_mb: 256,
            created_at: "2026-01-01T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string_pretty(&meta).unwrap();
        assert!(json.contains("\"name\": \"snap1\""));
        assert!(json.contains("\"sandbox\": \"sb1\""));
        assert!(json.contains("\"image_tag\": \"agentkernel-snap:snap1\""));
    }
}
