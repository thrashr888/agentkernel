//! Image cache management — list, prune, and pull Docker images.
//!
//! Provides visibility into disk usage from cached images and tools
//! to clean up unused images.

use anyhow::{Context, Result};
use std::process::Command;

/// Info about a Docker image.
#[derive(Debug, Clone)]
pub struct ImageInfo {
    pub repository: String,
    pub tag: String,
    pub image_id: String,
    pub size: String,
}

impl ImageInfo {
    pub fn full_name(&self) -> String {
        if self.tag == "<none>" {
            self.image_id.clone()
        } else {
            format!("{}:{}", self.repository, self.tag)
        }
    }
}

/// List all Docker images (optionally filtered to agentkernel-related).
pub fn list_images(all: bool) -> Result<Vec<ImageInfo>> {
    let output = Command::new("docker")
        .args([
            "images",
            "--format",
            "{{.Repository}}\t{{.Tag}}\t{{.ID}}\t{{.Size}}",
        ])
        .output()
        .context("Failed to run docker images")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("docker images failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut images: Vec<ImageInfo> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 4 {
                Some(ImageInfo {
                    repository: parts[0].to_string(),
                    tag: parts[1].to_string(),
                    image_id: parts[2].to_string(),
                    size: parts[3].to_string(),
                })
            } else {
                None
            }
        })
        .collect();

    if !all {
        // Filter to agentkernel-related images
        images.retain(|img| {
            img.repository.starts_with("agentkernel")
                || img.repository.contains("agentkernel")
                || img.tag.starts_with("agentkernel")
        });
    }

    Ok(images)
}

/// Count how many sandboxes use a given image.
pub fn sandbox_usage(image_name: &str) -> Result<usize> {
    let data_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".local/share/agentkernel/sandboxes");

    if !data_dir.exists() {
        return Ok(0);
    }

    let mut count = 0;
    for entry in std::fs::read_dir(&data_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json")
            && let Ok(content) = std::fs::read_to_string(&path)
            && content.contains(image_name)
        {
            count += 1;
        }
    }
    Ok(count)
}

/// Prune unused Docker images (dangling or agentkernel-specific).
pub fn prune(agentkernel_only: bool) -> Result<String> {
    if agentkernel_only {
        // Remove agentkernel-built images not used by any sandbox
        let images = list_images(false)?;
        let mut removed = 0;
        for img in &images {
            if sandbox_usage(&img.full_name())? == 0 {
                let output = Command::new("docker").args(["rmi", &img.image_id]).output();
                if let Ok(o) = output
                    && o.status.success()
                {
                    removed += 1;
                }
            }
        }
        Ok(format!("{} agentkernel image(s) removed", removed))
    } else {
        // Standard Docker image prune (dangling images)
        let output = Command::new("docker")
            .args(["image", "prune", "-f"])
            .output()
            .context("Failed to run docker image prune")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("docker image prune failed: {}", stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Pull a Docker image.
pub fn pull(image: &str) -> Result<()> {
    let status = Command::new("docker")
        .args(["pull", image])
        .status()
        .context("Failed to run docker pull")?;

    if !status.success() {
        anyhow::bail!("docker pull failed for '{}'", image);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_info_full_name() {
        let img = ImageInfo {
            repository: "python".to_string(),
            tag: "3.12-alpine".to_string(),
            image_id: "abc123".to_string(),
            size: "45MB".to_string(),
        };
        assert_eq!(img.full_name(), "python:3.12-alpine");
    }

    #[test]
    fn test_image_info_full_name_none_tag() {
        let img = ImageInfo {
            repository: "<none>".to_string(),
            tag: "<none>".to_string(),
            image_id: "abc123".to_string(),
            size: "45MB".to_string(),
        };
        assert_eq!(img.full_name(), "abc123");
    }

    #[test]
    fn test_sandbox_usage_no_dir() {
        // Should return 0 when sandbox dir doesn't exist
        let count = sandbox_usage("nonexistent-image:latest");
        assert!(count.is_ok());
    }
}
