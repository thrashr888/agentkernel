//! Docker image to ext4 rootfs conversion for Firecracker.
//!
//! Converts Docker images built from Dockerfiles into ext4 rootfs images
//! that can be used with Firecracker microVMs.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Result of a rootfs conversion
#[derive(Debug)]
pub struct ConversionResult {
    /// Path to the generated ext4 rootfs
    pub rootfs_path: PathBuf,
    /// Size of the rootfs in MB
    #[allow(dead_code)]
    pub size_mb: u64,
}

/// Convert a Docker image to an ext4 rootfs for Firecracker
///
/// This function:
/// 1. Exports the Docker image using `docker save`
/// 2. Runs a privileged Docker container to create the ext4 filesystem
/// 3. Extracts the image layers into the filesystem
/// 4. Injects the guest agent binary
/// 5. Creates the init script
///
/// # Arguments
/// * `image` - Docker image name/tag to convert
/// * `output_dir` - Directory to store the output rootfs
/// * `guest_agent_path` - Path to the guest agent binary (if None, uses default location)
///
/// # Returns
/// * `ConversionResult` with the rootfs path and size
pub fn convert_image_to_rootfs(
    image: &str,
    output_dir: &Path,
    guest_agent_path: Option<&Path>,
) -> Result<ConversionResult> {
    // Create output directory if it doesn't exist
    std::fs::create_dir_all(output_dir)?;

    // Generate rootfs filename from image name
    let rootfs_name = image_to_rootfs_name(image);
    let rootfs_path = output_dir.join(&rootfs_name);

    // Check if rootfs already exists (caching)
    if rootfs_path.exists() {
        let metadata = std::fs::metadata(&rootfs_path)?;
        eprintln!("Using cached rootfs: {}", rootfs_path.display());
        return Ok(ConversionResult {
            rootfs_path,
            size_mb: metadata.len() / (1024 * 1024),
        });
    }

    eprintln!("Converting Docker image '{}' to rootfs...", image);

    // Find the guest agent binary
    let agent_path = find_guest_agent(guest_agent_path)?;
    eprintln!("  Using guest agent: {}", agent_path.display());

    // Create a temporary directory for the conversion
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
    let temp_path = temp_dir.path();

    // Step 1: Export the Docker image
    let image_tar = temp_path.join("image.tar");
    eprintln!("  Exporting Docker image...");
    export_docker_image(image, &image_tar)?;

    // Step 2: Run the conversion in a privileged Docker container
    eprintln!("  Creating ext4 rootfs (256MB)...");
    let size_mb = 256u64;
    run_conversion_container(&image_tar, &rootfs_path, &agent_path, size_mb)?;

    eprintln!(
        "  Rootfs created: {} ({}MB)",
        rootfs_path.display(),
        size_mb
    );

    Ok(ConversionResult {
        rootfs_path,
        size_mb,
    })
}

/// Generate a rootfs filename from a Docker image name
fn image_to_rootfs_name(image: &str) -> String {
    // Replace characters that aren't filesystem-safe
    let safe_name = image.replace(['/', ':', '@'], "-");
    format!("{}.ext4", safe_name)
}

/// Find the guest agent binary
fn find_guest_agent(explicit_path: Option<&Path>) -> Result<PathBuf> {
    // Use explicit path if provided
    if let Some(path) = explicit_path {
        if path.exists() {
            return Ok(path.to_path_buf());
        }
        bail!("Guest agent not found at: {}", path.display());
    }

    // Check development locations
    let dev_paths = [
        "images/rootfs/agent",
        "target/x86_64-unknown-linux-musl/release/agent",
    ];
    for path in dev_paths {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
    }

    // Check installed location
    if let Some(home) = std::env::var_os("HOME") {
        let installed = PathBuf::from(home).join(".local/share/agentkernel/bin/agent");
        if installed.exists() {
            return Ok(installed);
        }
    }

    bail!(
        "Guest agent binary not found. Build it with:\n\
         cd guest-agent && cargo build --release --target x86_64-unknown-linux-musl"
    )
}

/// Export a Docker image to a tar file
fn export_docker_image(image: &str, output: &Path) -> Result<()> {
    let output_str = output.to_string_lossy();

    let result = Command::new("docker")
        .args(["save", "-o", &output_str, image])
        .output()
        .context("Failed to run docker save")?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        bail!("docker save failed: {}", stderr);
    }

    Ok(())
}

/// Run the conversion in a privileged Docker container
///
/// This container:
/// 1. Creates an ext4 image file
/// 2. Mounts it via loop device
/// 3. Extracts the Docker image layers
/// 4. Injects the guest agent and init script
fn run_conversion_container(
    image_tar: &Path,
    output_rootfs: &Path,
    agent_path: &Path,
    size_mb: u64,
) -> Result<()> {
    // The conversion script that runs inside the container
    let script = format!(
        r#"
set -euo pipefail

# Install required tools
apk add --no-cache e2fsprogs >/dev/null 2>&1

# Create ext4 image
dd if=/dev/zero of=/output/rootfs.ext4 bs=1M count={size_mb} status=none
mkfs.ext4 -F -q /output/rootfs.ext4

# Mount the rootfs
mkdir -p /mnt/rootfs
mount -o loop /output/rootfs.ext4 /mnt/rootfs

# Extract Docker image layers
mkdir -p /tmp/image
cd /tmp/image
tar xf /input/image.tar

# Find and extract layers in order (from manifest.json)
if [ -f manifest.json ]; then
    # Parse layers from manifest
    LAYERS=$(cat manifest.json | grep -o '"Layers":\[[^]]*\]' | grep -o '[^"]*\.tar' || true)
    for layer in $LAYERS; do
        if [ -f "$layer" ]; then
            tar xf "$layer" -C /mnt/rootfs 2>/dev/null || true
        fi
    done
else
    # Fallback: extract all layer.tar files
    for layer in */layer.tar; do
        if [ -f "$layer" ]; then
            tar xf "$layer" -C /mnt/rootfs 2>/dev/null || true
        fi
    done
fi

# Create essential directories
mkdir -p /mnt/rootfs/{{dev,proc,sys,tmp,run,root,app}}
chmod 1777 /mnt/rootfs/tmp

# Create device nodes
mknod -m 622 /mnt/rootfs/dev/console c 5 1 2>/dev/null || true
mknod -m 666 /mnt/rootfs/dev/null c 1 3 2>/dev/null || true
mknod -m 666 /mnt/rootfs/dev/zero c 1 5 2>/dev/null || true
mknod -m 666 /mnt/rootfs/dev/tty c 5 0 2>/dev/null || true
mknod -m 666 /mnt/rootfs/dev/random c 1 8 2>/dev/null || true
mknod -m 666 /mnt/rootfs/dev/urandom c 1 9 2>/dev/null || true

# Install guest agent
cp /input/agent /mnt/rootfs/usr/bin/agent
chmod +x /mnt/rootfs/usr/bin/agent

# Create init script
cat > /mnt/rootfs/init << 'INIT'
#!/bin/sh

# Mount essential filesystems
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev 2>/dev/null || true

# Set hostname
hostname agentkernel

# Start guest agent in background
/usr/bin/agent &

echo "Agentkernel guest ready"

# If no arguments, run shell (for debugging)
if [ $# -eq 0 ]; then
    exec /bin/sh
else
    exec "$@"
fi
INIT
chmod +x /mnt/rootfs/init

# Set up /etc files if not present
if [ ! -f /mnt/rootfs/etc/hostname ]; then
    echo "agentkernel" > /mnt/rootfs/etc/hostname
fi

# Unmount
umount /mnt/rootfs

echo "Conversion complete"
"#,
        size_mb = size_mb
    );

    // Get absolute paths for mounts
    let image_tar_abs = image_tar
        .canonicalize()
        .context("Failed to get absolute path for image tar")?;
    let agent_abs = agent_path
        .canonicalize()
        .context("Failed to get absolute path for guest agent")?;
    let output_dir = output_rootfs
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid output path"))?;

    // Create output directory
    std::fs::create_dir_all(output_dir)?;
    let output_dir_abs = output_dir
        .canonicalize()
        .context("Failed to get absolute path for output directory")?;

    // Run the conversion container
    let result = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--privileged",
            "-v",
            &format!("{}:/input/image.tar:ro", image_tar_abs.display()),
            "-v",
            &format!("{}:/input/agent:ro", agent_abs.display()),
            "-v",
            &format!("{}:/output", output_dir_abs.display()),
            "alpine:3.20",
            "sh",
            "-c",
            &script,
        ])
        .output()
        .context("Failed to run conversion container")?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        bail!(
            "Rootfs conversion failed:\nstdout: {}\nstderr: {}",
            stdout,
            stderr
        );
    }

    // Rename the output file to the final name
    let temp_rootfs = output_dir_abs.join("rootfs.ext4");
    if temp_rootfs.exists() {
        std::fs::rename(&temp_rootfs, output_rootfs).context("Failed to rename rootfs file")?;
    }

    Ok(())
}

/// Check if a rootfs conversion is needed for an image
#[allow(dead_code)]
pub fn needs_conversion(image: &str, output_dir: &Path) -> bool {
    let rootfs_name = image_to_rootfs_name(image);
    let rootfs_path = output_dir.join(&rootfs_name);
    !rootfs_path.exists()
}

/// Get the rootfs path for an image (without converting)
#[allow(dead_code)]
pub fn rootfs_path_for_image(image: &str, output_dir: &Path) -> PathBuf {
    let rootfs_name = image_to_rootfs_name(image);
    output_dir.join(&rootfs_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_to_rootfs_name() {
        assert_eq!(image_to_rootfs_name("alpine:3.20"), "alpine-3.20.ext4");
        assert_eq!(
            image_to_rootfs_name("my-app/image:latest"),
            "my-app-image-latest.ext4"
        );
        assert_eq!(
            image_to_rootfs_name("agentkernel-project:abc123"),
            "agentkernel-project-abc123.ext4"
        );
    }

    #[test]
    fn test_needs_conversion() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Should need conversion (file doesn't exist)
        assert!(needs_conversion("test:latest", temp_dir.path()));

        // Create the file
        let rootfs_path = temp_dir.path().join("test-latest.ext4");
        std::fs::write(&rootfs_path, "fake rootfs").unwrap();

        // Should not need conversion (file exists)
        assert!(!needs_conversion("test:latest", temp_dir.path()));
    }
}
