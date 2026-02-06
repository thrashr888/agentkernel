//! Custom image builder for sandboxes.
//!
//! Build custom images from Dockerfiles and store them locally
//! for use with agentkernel sandboxes.
//!
//! # Example
//!
//! ```bash
//! # Build a custom image
//! agentkernel build -t my-tools .
//!
//! # Use it in a sandbox
//! agentkernel create my-sandbox --image my-tools
//! ```

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Metadata for a locally built image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalImage {
    /// Image name/tag (e.g., "my-tools")
    pub name: String,
    /// Docker image ID (sha256:...)
    pub image_id: String,
    /// Size in bytes
    pub size_bytes: u64,
    /// When the image was built (RFC3339)
    pub built_at: String,
    /// Dockerfile path used to build
    pub dockerfile: Option<String>,
    /// Build context path
    pub context: Option<String>,
    /// Labels from the image
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

impl LocalImage {
    /// Format size for display.
    pub fn format_size(&self) -> String {
        if self.size_bytes >= 1024 * 1024 * 1024 {
            format!(
                "{:.1}GB",
                self.size_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
            )
        } else if self.size_bytes >= 1024 * 1024 {
            format!("{:.1}MB", self.size_bytes as f64 / (1024.0 * 1024.0))
        } else if self.size_bytes >= 1024 {
            format!("{:.1}KB", self.size_bytes as f64 / 1024.0)
        } else {
            format!("{}B", self.size_bytes)
        }
    }

    /// Get the full Docker image reference.
    pub fn docker_ref(&self) -> String {
        format!("agentkernel-{}", self.name)
    }
}

/// Manages locally built images.
pub struct ImageBuilder {
    /// Directory where image metadata is stored
    metadata_dir: PathBuf,
    /// Loaded images
    images: HashMap<String, LocalImage>,
}

impl ImageBuilder {
    /// Create a new image builder.
    pub fn new() -> Result<Self> {
        let metadata_dir = Self::base_dir().join("image-metadata");
        std::fs::create_dir_all(&metadata_dir)
            .context("Failed to create image metadata directory")?;

        let mut builder = Self {
            metadata_dir,
            images: HashMap::new(),
        };
        builder.load_all()?;
        Ok(builder)
    }

    /// Get the base data directory.
    fn base_dir() -> PathBuf {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".agentkernel")
        } else {
            PathBuf::from("/tmp/agentkernel")
        }
    }

    /// Load all image metadata from disk.
    fn load_all(&mut self) -> Result<()> {
        if !self.metadata_dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&self.metadata_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && let Ok(content) = std::fs::read_to_string(&path)
                && let Ok(image) = serde_json::from_str::<LocalImage>(&content)
            {
                self.images.insert(image.name.clone(), image);
            }
        }
        Ok(())
    }

    /// Save image metadata to disk.
    fn save(&self, image: &LocalImage) -> Result<()> {
        let path = self.metadata_dir.join(format!("{}.json", image.name));
        let content = serde_json::to_string_pretty(image)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Build an image from a Dockerfile.
    ///
    /// # Arguments
    /// * `name` - Name for the built image
    /// * `context` - Build context directory
    /// * `dockerfile` - Optional path to Dockerfile (defaults to "Dockerfile" in context)
    pub fn build(
        &mut self,
        name: &str,
        context: &Path,
        dockerfile: Option<&Path>,
    ) -> Result<LocalImage> {
        // Validate name
        if name.is_empty() {
            bail!("Image name cannot be empty");
        }
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            bail!(
                "Image name can only contain alphanumeric characters, hyphens, underscores, and dots"
            );
        }

        // Validate context
        if !context.exists() {
            bail!("Build context '{}' does not exist", context.display());
        }

        // Find Dockerfile
        let dockerfile_path = if let Some(df) = dockerfile {
            if !df.exists() {
                bail!("Dockerfile '{}' does not exist", df.display());
            }
            df.to_path_buf()
        } else {
            let default_dockerfile = context.join("Dockerfile");
            if !default_dockerfile.exists() {
                bail!(
                    "No Dockerfile found in '{}'. Specify one with --dockerfile",
                    context.display()
                );
            }
            default_dockerfile
        };

        let docker_tag = format!("agentkernel-{}", name);

        // Build with Docker
        eprintln!("Building image '{}'...", name);
        let mut args = vec![
            "build".to_string(),
            "-t".to_string(),
            docker_tag.clone(),
            "-f".to_string(),
            dockerfile_path.to_string_lossy().to_string(),
        ];

        // Add agentkernel label
        args.push("--label".to_string());
        args.push("agentkernel.managed=true".to_string());
        args.push("--label".to_string());
        args.push(format!("agentkernel.name={}", name));

        args.push(context.to_string_lossy().to_string());

        let output = Command::new("docker")
            .args(&args)
            .output()
            .context("Failed to run docker build")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Docker build failed:\n{}", stderr);
        }

        // Get image info
        let inspect_output = Command::new("docker")
            .args(["inspect", "--format", "{{.Id}} {{.Size}}", &docker_tag])
            .output()
            .context("Failed to inspect built image")?;

        if !inspect_output.status.success() {
            bail!("Failed to get image info after build");
        }

        let inspect_str = String::from_utf8_lossy(&inspect_output.stdout);
        let parts: Vec<&str> = inspect_str.split_whitespace().collect();

        let image_id = parts.first().unwrap_or(&"").to_string();
        let size_bytes: u64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);

        let image = LocalImage {
            name: name.to_string(),
            image_id,
            size_bytes,
            built_at: chrono::Utc::now().to_rfc3339(),
            dockerfile: Some(dockerfile_path.to_string_lossy().to_string()),
            context: Some(context.to_string_lossy().to_string()),
            labels: HashMap::new(),
        };

        self.save(&image)?;
        self.images.insert(name.to_string(), image.clone());

        eprintln!("Built image '{}' ({})", name, image.format_size());
        Ok(image)
    }

    /// Get an image by name.
    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Option<&LocalImage> {
        self.images.get(name)
    }

    /// Check if an image exists locally.
    #[allow(dead_code)]
    pub fn exists(&self, name: &str) -> bool {
        self.images.contains_key(name)
    }

    /// List all locally built images.
    pub fn list(&self) -> Vec<&LocalImage> {
        self.images.values().collect()
    }

    /// Delete a locally built image.
    pub fn delete(&mut self, name: &str) -> Result<()> {
        let image = self
            .images
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Image '{}' not found", name))?;

        let docker_tag = image.docker_ref();

        // Remove Docker image
        let output = Command::new("docker")
            .args(["rmi", &docker_tag])
            .output()
            .context("Failed to run docker rmi")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "image not found" errors
            if !stderr.contains("No such image") {
                bail!("Failed to remove Docker image: {}", stderr.trim());
            }
        }

        // Remove metadata file
        let metadata_path = self.metadata_dir.join(format!("{}.json", name));
        if metadata_path.exists() {
            std::fs::remove_file(&metadata_path).context("Failed to remove image metadata")?;
        }

        self.images.remove(name);
        Ok(())
    }

    /// Resolve an image name to a Docker reference.
    ///
    /// Returns the agentkernel-prefixed name for local images,
    /// or the original name for remote images.
    #[allow(dead_code)]
    pub fn resolve_image(&self, name: &str) -> String {
        if self.exists(name) {
            format!("agentkernel-{}", name)
        } else {
            name.to_string()
        }
    }

    /// Sync metadata with actual Docker images.
    ///
    /// Removes metadata for images that no longer exist in Docker.
    pub fn sync(&mut self) -> Result<Vec<String>> {
        let mut removed = Vec::new();

        let names: Vec<String> = self.images.keys().cloned().collect();
        for name in names {
            let docker_tag = format!("agentkernel-{}", name);

            // Check if image exists in Docker
            let output = Command::new("docker")
                .args(["inspect", &docker_tag])
                .output();

            if let Ok(out) = output
                && !out.status.success()
            {
                // Image doesn't exist, remove metadata
                let metadata_path = self.metadata_dir.join(format!("{}.json", name));
                if metadata_path.exists() {
                    let _ = std::fs::remove_file(&metadata_path);
                }
                self.images.remove(&name);
                removed.push(name);
            }
        }

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_image_format_size() {
        let image = LocalImage {
            name: "test".to_string(),
            image_id: "sha256:abc".to_string(),
            size_bytes: 1024 * 1024 * 100, // 100MB
            built_at: "2024-01-01T00:00:00Z".to_string(),
            dockerfile: None,
            context: None,
            labels: HashMap::new(),
        };
        assert_eq!(image.format_size(), "100.0MB");
    }

    #[test]
    fn test_local_image_docker_ref() {
        let image = LocalImage {
            name: "my-tools".to_string(),
            image_id: "sha256:abc".to_string(),
            size_bytes: 0,
            built_at: "2024-01-01T00:00:00Z".to_string(),
            dockerfile: None,
            context: None,
            labels: HashMap::new(),
        };
        assert_eq!(image.docker_ref(), "agentkernel-my-tools");
    }

    #[test]
    fn test_image_builder_resolve_image() {
        // Without actually building, test the resolve logic
        let builder = ImageBuilder {
            metadata_dir: PathBuf::from("/tmp"),
            images: {
                let mut m = HashMap::new();
                m.insert(
                    "local-image".to_string(),
                    LocalImage {
                        name: "local-image".to_string(),
                        image_id: "sha256:abc".to_string(),
                        size_bytes: 0,
                        built_at: "2024-01-01T00:00:00Z".to_string(),
                        dockerfile: None,
                        context: None,
                        labels: HashMap::new(),
                    },
                );
                m
            },
        };

        // Local image gets prefixed
        assert_eq!(
            builder.resolve_image("local-image"),
            "agentkernel-local-image"
        );

        // Remote image stays as-is
        assert_eq!(
            builder.resolve_image("python:3.12-alpine"),
            "python:3.12-alpine"
        );
    }
}
