//! Snapshot and restore for sandboxes.
//!
//! Save sandbox state (Docker: `docker commit` + metadata) and restore later.
//! Snapshots are stored in `~/.local/share/agentkernel/snapshots/`.

use crate::backend::{
    BackendType,
    remote::{remote_bridge_command, remote_bridge_env},
};
use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

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
    /// Opaque remote-provider snapshot handle for remote backends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_snapshot: Option<String>,
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

/// Take a snapshot of a running or stopped container.
///
/// For Docker backend: uses `docker commit`.
/// For Apple backend: exports the container filesystem and builds a native image.
pub fn take(
    sandbox_name: &str,
    snapshot_name: &str,
    state: &SnapshotInput,
) -> Result<SnapshotMeta> {
    let backend = state
        .backend
        .parse::<BackendType>()
        .unwrap_or(BackendType::Docker);
    let (image_tag, remote_snapshot) = if backend.is_remote() {
        (
            state.image.clone(),
            Some(take_remote_snapshot(sandbox_name, snapshot_name, state)?),
        )
    } else {
        let image_tag = format!("agentkernel-snap:{}", snapshot_name);
        let container_name = format!("agentkernel-{}", sandbox_name);

        match state.backend.as_str() {
            "apple" => take_apple(&container_name, &image_tag)?,
            _ => take_docker(&container_name, &image_tag)?,
        }

        (image_tag, None)
    };

    let meta = SnapshotMeta {
        name: snapshot_name.to_string(),
        sandbox: sandbox_name.to_string(),
        image_tag,
        backend: state.backend.clone(),
        base_image: state.image.clone(),
        vcpus: state.vcpus,
        memory_mb: state.memory_mb,
        created_at: chrono::Utc::now().to_rfc3339(),
        remote_snapshot,
    };

    // Save metadata
    let dir = snapshots_dir();
    std::fs::create_dir_all(&dir)?;
    let meta_path = dir.join(format!("{}.json", snapshot_name));
    let content = serde_json::to_string_pretty(&meta)?;
    std::fs::write(&meta_path, content)?;

    Ok(meta)
}

fn take_docker(container_name: &str, image_tag: &str) -> Result<()> {
    let output = std::process::Command::new("docker")
        .args(["commit", container_name, image_tag])
        .output()
        .context("Failed to run docker commit")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("docker commit failed: {}", stderr.trim());
    }
    Ok(())
}

fn take_apple(container_name: &str, image_tag: &str) -> Result<()> {
    use std::io::Write;
    use std::process::Stdio;

    let tmp = tempfile::tempdir().context("Failed to create temp dir")?;
    let rootfs_path = tmp.path().join("rootfs.tar");

    // Create tarball inside the container by archiving top-level directories
    // individually (avoids BusyBox tar issues with --exclude on virtual filesystems).
    let tar_inside = "/tmp/_ak_snapshot.tar";

    // List top-level directories, skipping virtual filesystems
    let ls_out = std::process::Command::new("container")
        .args(["exec", container_name, "ls", "-1", "/"])
        .output()
        .context("Failed to list container root")?;
    let ls_text = String::from_utf8_lossy(&ls_out.stdout);
    let skip = ["proc", "sys", "dev", "tmp"];
    let dirs: Vec<String> = ls_text
        .lines()
        .filter(|d| !d.is_empty() && !skip.contains(&d.trim()))
        .map(|d| format!("/{}", d.trim()))
        .collect();

    if dirs.is_empty() {
        bail!("No directories to snapshot in container");
    }

    let mut tar_args = vec![
        "exec".to_string(),
        container_name.to_string(),
        "tar".to_string(),
        "cf".to_string(),
        tar_inside.to_string(),
    ];
    tar_args.extend(dirs);

    let create = std::process::Command::new("container")
        .args(&tar_args)
        .stderr(Stdio::piped())
        .output()
        .context("Failed to create tarball inside container")?;

    // tar exit code 1 = "some files changed" (acceptable); only fail on 2+
    if !create.status.success() && create.status.code().unwrap_or(2) >= 2 {
        let stderr = String::from_utf8_lossy(&create.stderr);
        bail!("tar inside container failed: {}", stderr.trim());
    }

    // Read the tarball back out
    let cat_proc = std::process::Command::new("container")
        .args(["exec", container_name, "cat", tar_inside])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to read tarball from container")?;

    if cat_proc.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&cat_proc.stderr);
        bail!(
            "Failed to read tarball from container (empty): {}",
            stderr.trim()
        );
    }

    std::fs::write(&rootfs_path, &cat_proc.stdout).context("Failed to write rootfs tarball")?;

    // Clean up tar inside container
    let _ = std::process::Command::new("container")
        .args(["exec", container_name, "rm", "-f", tar_inside])
        .output();

    // Write Dockerfile
    let dockerfile_path = tmp.path().join("Dockerfile");
    let mut f = std::fs::File::create(&dockerfile_path).context("Failed to create Dockerfile")?;
    writeln!(f, "FROM scratch")?;
    writeln!(f, "ADD rootfs.tar /")?;
    writeln!(f, "CMD [\"sleep\", \"infinity\"]")?;

    // Build image
    let build = std::process::Command::new("container")
        .args(["build", "-t", image_tag, &tmp.path().to_string_lossy()])
        .output()
        .context("Failed to build snapshot image")?;

    if !build.status.success() {
        let stderr = String::from_utf8_lossy(&build.stderr);
        bail!("container build failed: {}", stderr.trim());
    }

    Ok(())
}

/// Input needed from the sandbox state to create a snapshot.
pub struct SnapshotInput {
    pub image: String,
    pub backend: String,
    pub vcpus: u32,
    pub memory_mb: u64,
    pub remote_id: Option<String>,
    pub remote_namespace: Option<String>,
    pub remote_metadata: HashMap<String, String>,
    pub workspace_revision: Option<String>,
}

impl SnapshotMeta {
    pub fn restore_image(&self) -> &str {
        if self.remote_snapshot.is_some() {
            &self.base_image
        } else {
            &self.image_tag
        }
    }
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

/// Delete a snapshot (removes metadata and image from the appropriate store).
pub fn delete(name: &str) -> Result<()> {
    let meta = get(name)?.ok_or_else(|| anyhow::anyhow!("Snapshot '{}' not found", name))?;

    // Remove image from the backend that created it
    if let Some(snapshot_handle) = meta.remote_snapshot.as_deref() {
        let _ = delete_remote_snapshot(&meta, snapshot_handle);
    } else {
        match meta.backend.as_str() {
            "apple" => {
                let _ = std::process::Command::new("container")
                    .args(["image", "rm", &meta.image_tag])
                    .output();
            }
            _ => {
                let _ = std::process::Command::new("docker")
                    .args(["rmi", &meta.image_tag])
                    .output();
            }
        }
    }

    // Remove metadata file
    let path = snapshots_dir().join(format!("{}.json", name));
    if path.exists() {
        std::fs::remove_file(&path)?;
    }

    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct RemoteSnapshotRequest {
    operation: String,
    sandbox_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote_namespace: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    remote_metadata: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteSnapshotResponse {
    success: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    remote_metadata: HashMap<String, String>,
}

fn take_remote_snapshot(
    sandbox_name: &str,
    snapshot_name: &str,
    state: &SnapshotInput,
) -> Result<String> {
    let response = remote_snapshot_request(
        &state.backend,
        &RemoteSnapshotRequest {
            operation: "snapshot".to_string(),
            sandbox_name: sandbox_name.to_string(),
            remote_id: state.remote_id.clone(),
            remote_namespace: state.remote_namespace.clone(),
            remote_metadata: state.remote_metadata.clone(),
            workspace_revision: state.workspace_revision.clone(),
            snapshot_name: Some(snapshot_name.to_string()),
        },
    )?;

    response
        .remote_metadata
        .get("snapshot_handle")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Remote backend did not return a snapshot handle"))
}

fn delete_remote_snapshot(meta: &SnapshotMeta, snapshot_handle: &str) -> Result<()> {
    let _ = remote_snapshot_request(
        &meta.backend,
        &RemoteSnapshotRequest {
            operation: "delete_snapshot".to_string(),
            sandbox_name: meta.sandbox.clone(),
            remote_id: None,
            remote_namespace: None,
            remote_metadata: HashMap::new(),
            workspace_revision: None,
            snapshot_name: Some(snapshot_handle.to_string()),
        },
    )?;
    Ok(())
}

fn remote_snapshot_request(
    provider: &str,
    request: &RemoteSnapshotRequest,
) -> Result<RemoteSnapshotResponse> {
    let payload = STANDARD.encode(
        serde_json::to_vec(request).context("Failed to serialize remote snapshot request")?,
    );

    let (program, mut args) = remote_bridge_command()?;
    args.push(provider.to_string());
    args.push(payload);

    let output = std::process::Command::new(program)
        .args(args)
        .envs(remote_bridge_env(provider))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Failed to execute remote bridge")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if !stderr.is_empty() { stderr } else { stdout };
        bail!("Remote bridge request failed for {}: {}", provider, message);
    }

    let stdout = String::from_utf8(output.stdout).context("Remote bridge returned non-utf8")?;
    let response: RemoteSnapshotResponse =
        serde_json::from_str(stdout.trim()).with_context(|| {
            format!(
                "Failed to parse remote bridge snapshot response for {}",
                provider
            )
        })?;

    if !response.success {
        bail!(
            "{} backend error: {}",
            provider,
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }

    Ok(response)
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
            remote_snapshot: None,
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
            remote_snapshot: None,
        };

        let json = serde_json::to_string_pretty(&meta).unwrap();
        assert!(json.contains("\"name\": \"snap1\""));
        assert!(json.contains("\"sandbox\": \"sb1\""));
        assert!(json.contains("\"image_tag\": \"agentkernel-snap:snap1\""));
    }

    #[test]
    fn test_remote_snapshot_uses_base_image_for_restore() {
        let meta = SnapshotMeta {
            name: "remote-snap".to_string(),
            sandbox: "remote-sandbox".to_string(),
            image_tag: "ignored-for-remote".to_string(),
            backend: "daytona".to_string(),
            base_image: "default".to_string(),
            vcpus: 1,
            memory_mb: 256,
            created_at: "2026-03-24T00:00:00Z".to_string(),
            remote_snapshot: Some("remote-1:remote-snap".to_string()),
        };

        assert_eq!(meta.restore_image(), "default");
    }
}
