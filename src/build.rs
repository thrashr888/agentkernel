//! Docker image building for custom Dockerfiles.
//!
//! Provides functionality to build Docker images from Dockerfiles
//! with caching support based on content hashing.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

use crate::config::BuildConfig;
use crate::docker_backend::{ContainerRuntime, docker_available, podman_available};
use crate::languages::dockerfile_image_name;

/// Result of a Docker build operation
#[derive(Debug)]
pub struct BuildResult {
    /// The image name/tag that was built
    pub image: String,
    /// Whether the image was already cached (no build needed)
    #[allow(dead_code)]
    pub cached: bool,
}

/// Check if a Docker image exists locally
pub fn image_exists(image: &str, runtime: ContainerRuntime) -> bool {
    let cmd = runtime.cmd();
    Command::new(cmd)
        .args(["image", "inspect", image])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a Docker image from a Dockerfile
///
/// # Arguments
/// * `project_name` - Name used for the image tag
/// * `dockerfile_path` - Path to the Dockerfile
/// * `context_path` - Build context directory
/// * `config` - Build configuration (args, target, no_cache)
///
/// # Returns
/// * `BuildResult` with the image name and whether it was cached
pub fn build_image(
    project_name: &str,
    dockerfile_path: &Path,
    context_path: &Path,
    config: &BuildConfig,
) -> Result<BuildResult> {
    // Determine which runtime to use
    let runtime = if docker_available() {
        ContainerRuntime::Docker
    } else if podman_available() {
        ContainerRuntime::Podman
    } else {
        bail!("No container runtime available (need Docker or Podman)");
    };

    // Generate deterministic image name based on content hash
    let image_name = dockerfile_image_name(project_name, dockerfile_path);

    // Check if image already exists (cached)
    if !config.no_cache && image_exists(&image_name, runtime) {
        eprintln!("Using cached image: {}", image_name);
        return Ok(BuildResult {
            image: image_name,
            cached: true,
        });
    }

    eprintln!("Building image from {}...", dockerfile_path.display());

    // Build the docker build command
    let mut args = vec![
        "build".to_string(),
        "-t".to_string(),
        image_name.clone(),
        "-f".to_string(),
        dockerfile_path.to_string_lossy().to_string(),
    ];

    // Add build target if specified
    if let Some(ref target) = config.target {
        args.push("--target".to_string());
        args.push(target.clone());
    }

    // Add build args
    for (key, value) in &config.args {
        args.push("--build-arg".to_string());
        args.push(format!("{}={}", key, value));
    }

    // Add no-cache flag if requested
    if config.no_cache {
        args.push("--no-cache".to_string());
    }

    // Add context path
    args.push(context_path.to_string_lossy().to_string());

    // Run the build
    let output = Command::new(runtime.cmd())
        .args(&args)
        .output()
        .context("Failed to run docker build")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Docker build failed:\n{}", stderr);
    }

    eprintln!("Built image: {}", image_name);

    Ok(BuildResult {
        image: image_name,
        cached: false,
    })
}

/// Build image if Dockerfile exists, otherwise return the base image
///
/// This is the main entry point for the build system. It handles:
/// - Auto-detection of Dockerfiles
/// - Building with caching
/// - Falling back to base image if no Dockerfile
pub fn build_or_use_image(
    project_name: &str,
    base_image: &str,
    base_dir: &Path,
    config: &crate::config::Config,
) -> Result<String> {
    // Check if we need to build
    if let Some(dockerfile_path) = config.dockerfile_path(base_dir) {
        let context_path = config.build_context(base_dir, &dockerfile_path);
        let result = build_image(project_name, &dockerfile_path, &context_path, &config.build)?;
        Ok(result.image)
    } else {
        // No Dockerfile, use the base image
        Ok(base_image.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_image_name_generation() {
        let dir = tempdir().unwrap();
        let dockerfile_path = dir.path().join("Dockerfile");

        let mut file = std::fs::File::create(&dockerfile_path).unwrap();
        writeln!(file, "FROM alpine:3.20").unwrap();

        let name = dockerfile_image_name("my-project", &dockerfile_path);
        assert!(name.starts_with("agentkernel-my-project:"));
    }
}
