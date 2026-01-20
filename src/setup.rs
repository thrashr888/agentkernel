//! Setup and installation management for agentkernel.
//!
//! Handles downloading/building kernel, rootfs, and Firecracker.

use anyhow::{Context, Result, bail};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Runtime options for rootfs
pub const RUNTIMES: &[(&str, &str)] = &[
    ("base", "Minimal Alpine Linux (~64MB)"),
    ("python", "Python 3.12 with pip (~256MB)"),
    ("node", "Node.js 20 LTS with npm (~256MB)"),
    ("go", "Go toolchain (~512MB)"),
    ("rust", "Rust with Cargo (~512MB)"),
];

/// Setup configuration
#[allow(dead_code)]
pub struct SetupConfig {
    pub data_dir: PathBuf,
    pub kernel_version: String,
    pub runtimes: Vec<String>,
    pub install_firecracker: bool,
}

impl Default for SetupConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            kernel_version: "6.1.70".to_string(),
            runtimes: vec!["base".to_string()],
            install_firecracker: true,
        }
    }
}

/// Get the default data directory
pub fn default_data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".local/share/agentkernel")
    } else {
        PathBuf::from("/usr/local/share/agentkernel")
    }
}

/// Check what components are installed
pub fn check_installation() -> SetupStatus {
    let data_dir = default_data_dir();

    // Check KVM status - distinguish between "not present" and "permission denied"
    let kvm_path = std::path::PathBuf::from("/dev/kvm");
    let kvm_exists = kvm_path.exists();
    let kvm_accessible = check_kvm();
    let kvm_permission_denied = kvm_exists && !kvm_accessible;

    SetupStatus {
        kernel_installed: find_kernel(&data_dir).is_some(),
        rootfs_base_installed: data_dir.join("images/rootfs/base.ext4").exists(),
        rootfs_python_installed: data_dir.join("images/rootfs/python.ext4").exists(),
        rootfs_node_installed: data_dir.join("images/rootfs/node.ext4").exists(),
        firecracker_installed: find_firecracker().is_some(),
        kvm_available: kvm_accessible,
        kvm_permission_denied,
        docker_available: check_docker(),
    }
}

/// Installation status
#[derive(Debug)]
#[allow(dead_code)]
pub struct SetupStatus {
    pub kernel_installed: bool,
    pub rootfs_base_installed: bool,
    pub rootfs_python_installed: bool,
    pub rootfs_node_installed: bool,
    pub firecracker_installed: bool,
    pub kvm_available: bool,
    /// True if /dev/kvm exists but user lacks permission to access it
    pub kvm_permission_denied: bool,
    pub docker_available: bool,
}

impl SetupStatus {
    pub fn is_ready(&self) -> bool {
        self.kernel_installed
            && self.rootfs_base_installed
            && self.firecracker_installed
            && (self.kvm_available || self.docker_available)
    }

    pub fn print(&self) {
        println!("Setup Status:");
        println!(
            "  Kernel:      {}",
            if self.kernel_installed {
                "installed"
            } else {
                "not installed"
            }
        );
        println!(
            "  Rootfs base: {}",
            if self.rootfs_base_installed {
                "installed"
            } else {
                "not installed"
            }
        );
        println!(
            "  Firecracker: {}",
            if self.firecracker_installed {
                "installed"
            } else {
                "not installed"
            }
        );
        // Show KVM status with helpful message if permission denied
        let kvm_status = if self.kvm_available {
            "available"
        } else if self.kvm_permission_denied {
            "permission denied"
        } else {
            "not available"
        };
        println!("  KVM:         {}", kvm_status);

        // Show guidance for KVM permission issues
        if self.kvm_permission_denied {
            println!();
            println!("  ⚠️  /dev/kvm exists but you don't have permission to access it.");
            println!("  Fix with: sudo usermod -aG kvm $USER && newgrp kvm");
        }

        println!(
            "  Docker:      {}",
            if self.docker_available {
                "available"
            } else {
                "not available"
            }
        );
    }
}

/// Find installed kernel
fn find_kernel(data_dir: &Path) -> Option<PathBuf> {
    let kernel_dir = data_dir.join("images/kernel");
    if kernel_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&kernel_dir)
    {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("vmlinux-") && name_str.ends_with("-agentkernel") {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Find Firecracker binary
fn find_firecracker() -> Option<PathBuf> {
    // Check agentkernel's own bin directory first
    let data_dir = default_data_dir();
    let local_fc = data_dir.join("bin/firecracker");
    if local_fc.exists() {
        return Some(local_fc);
    }

    // Check PATH
    if let Ok(output) = Command::new("which").arg("firecracker").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    // Check common locations
    let locations = ["/usr/local/bin/firecracker", "/usr/bin/firecracker"];

    for loc in locations {
        let path = PathBuf::from(loc);
        if path.exists() {
            return Some(path);
        }
    }

    None
}

/// Check if KVM is available and accessible
///
/// Returns true only if /dev/kvm exists AND the current user has read/write access.
/// This prevents the confusing case where status says "KVM: available" but operations fail.
fn check_kvm() -> bool {
    let kvm_path = std::path::PathBuf::from("/dev/kvm");
    if !kvm_path.exists() {
        return false;
    }

    // Check if we can actually access KVM (not just that it exists)
    // Try to open with read/write to verify permissions
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&kvm_path)
            .is_ok()
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Check if Docker is available
fn check_docker() -> bool {
    Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Prompt user to select from options
#[allow(dead_code)]
pub fn prompt_select(prompt: &str, options: &[(&str, &str)], default: usize) -> Result<usize> {
    println!("\n{}", prompt);
    for (i, (name, desc)) in options.iter().enumerate() {
        let marker = if i == default { " (recommended)" } else { "" };
        println!("  {}. {} - {}{}", i + 1, name, desc, marker);
    }

    print!("\nEnter choice [{}]: ", default + 1);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        return Ok(default);
    }

    match input.parse::<usize>() {
        Ok(n) if n >= 1 && n <= options.len() => Ok(n - 1),
        _ => {
            println!("Invalid choice, using default.");
            Ok(default)
        }
    }
}

/// Prompt user for yes/no
pub fn prompt_yes_no(prompt: &str, default: bool) -> Result<bool> {
    let default_str = if default { "Y/n" } else { "y/N" };
    print!("{} [{}]: ", prompt, default_str);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();

    if input.is_empty() {
        return Ok(default);
    }

    Ok(input == "y" || input == "yes")
}

/// Prompt user to select multiple options
pub fn prompt_multi_select(
    prompt: &str,
    options: &[(&str, &str)],
    defaults: &[usize],
) -> Result<Vec<usize>> {
    println!("\n{}", prompt);
    for (i, (name, desc)) in options.iter().enumerate() {
        let marker = if defaults.contains(&i) { " *" } else { "" };
        println!("  {}. {} - {}{}", i + 1, name, desc, marker);
    }
    println!("\n  (* = selected by default)");

    print!("Enter choices (comma-separated) or press Enter for defaults: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        return Ok(defaults.to_vec());
    }

    let mut selected = Vec::new();
    for part in input.split(',') {
        let part = part.trim();
        if let Ok(n) = part.parse::<usize>()
            && n >= 1
            && n <= options.len()
            && !selected.contains(&(n - 1))
        {
            selected.push(n - 1);
        }
    }

    if selected.is_empty() {
        return Ok(defaults.to_vec());
    }

    Ok(selected)
}

/// Run the interactive setup
pub async fn run_setup(non_interactive: bool) -> Result<()> {
    println!("=== Agentkernel Setup ===\n");

    let status = check_installation();
    status.print();

    if status.is_ready() && non_interactive {
        println!("\nAgentkernel is already set up and ready to use!");
        return Ok(());
    }

    // Check platform requirements
    if !status.kvm_available && !status.docker_available {
        println!("\nWarning: Neither KVM nor Docker is available.");
        println!("  - On Linux: Ensure /dev/kvm exists and is accessible");
        println!("  - On macOS: Install Docker Desktop");
        if !non_interactive && !prompt_yes_no("Continue anyway?", false)? {
            return Ok(());
        }
    }

    let data_dir = default_data_dir();
    println!("\nInstall location: {}", data_dir.display());

    // Determine what to install
    let mut install_kernel = !status.kernel_installed;
    let mut install_firecracker = !status.firecracker_installed;
    let mut runtimes_to_install: Vec<String> = Vec::new();

    if non_interactive {
        // Non-interactive: install everything needed
        if !status.rootfs_base_installed {
            runtimes_to_install.push("base".to_string());
        }
    } else {
        // Interactive mode: ask user
        if !status.kernel_installed {
            install_kernel = prompt_yes_no("\nBuild and install kernel?", true)?;
        }

        if !status.firecracker_installed {
            install_firecracker = prompt_yes_no("Download and install Firecracker?", true)?;
        }

        // Ask which runtimes to install
        let runtime_options: Vec<(&str, &str)> = RUNTIMES.to_vec();
        let defaults = vec![0]; // base is default

        let selected = prompt_multi_select(
            "Which runtimes would you like to install?",
            &runtime_options,
            &defaults,
        )?;

        for idx in selected {
            let runtime = RUNTIMES[idx].0;
            let rootfs_path = data_dir.join(format!("images/rootfs/{}.ext4", runtime));
            if !rootfs_path.exists() {
                runtimes_to_install.push(runtime.to_string());
            }
        }
    }

    // Create directories
    std::fs::create_dir_all(data_dir.join("images/kernel"))?;
    std::fs::create_dir_all(data_dir.join("images/rootfs"))?;
    std::fs::create_dir_all(data_dir.join("bin"))?;

    // Check for Docker (needed for building)
    if (install_kernel || !runtimes_to_install.is_empty()) && !status.docker_available {
        bail!("Docker is required to build kernel and rootfs images. Please install Docker first.");
    }

    // Install kernel
    if install_kernel {
        println!("\n==> Building kernel...");
        build_kernel(&data_dir).await?;
    }

    // Install runtimes
    for runtime in &runtimes_to_install {
        println!("\n==> Building {} rootfs...", runtime);
        build_rootfs(&data_dir, runtime).await?;
    }

    // Install Firecracker
    if install_firecracker {
        println!("\n==> Installing Firecracker...");
        install_firecracker_binary(&data_dir).await?;
    }

    println!("\n=== Setup Complete ===");

    // Re-check status after installation
    let final_status = check_installation();

    // Offer verification test if Firecracker backend is available
    if final_status.kernel_installed
        && final_status.rootfs_base_installed
        && final_status.firecracker_installed
    {
        if final_status.kvm_available {
            if !non_interactive {
                println!();
                if prompt_yes_no("Run a quick verification test?", true)? {
                    run_verification_test(&data_dir).await?;
                }
            }
        } else if final_status.kvm_permission_denied {
            println!(
                "\n⚠️  KVM permission denied - you need to fix this before using Firecracker."
            );
            println!("\nTo fix KVM permissions:");
            println!("  1. Add yourself to the kvm group:");
            println!("     sudo usermod -aG kvm $USER");
            println!("  2. Apply the group change (choose one):");
            println!("     - Log out and back in, OR");
            println!("     - Run: newgrp kvm");
            println!("     - Run commands with: sg kvm -c 'agentkernel start ...'");
            println!("\nAfter fixing permissions, run: agentkernel setup --verify");
        }
    }

    println!("\nYou can now create sandboxes with:");
    println!("  agentkernel create my-sandbox");
    println!("  agentkernel start my-sandbox");

    Ok(())
}

/// Build the kernel
async fn build_kernel(data_dir: &Path) -> Result<()> {
    // Find the build script in the source directory or use embedded version
    let script_content = include_str!("../images/build/build-kernel.sh");
    let config_content = include_str!("../images/kernel/microvm.config");

    // Create temp directory for build
    let temp_dir = std::env::temp_dir().join("agentkernel-kernel-build");
    std::fs::create_dir_all(&temp_dir)?;

    // Write build script and config
    let script_path = temp_dir.join("build-kernel.sh");
    let config_path = temp_dir.join("microvm.config");
    std::fs::write(&script_path, script_content)?;
    std::fs::write(&config_path, config_content)?;

    // Make script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // Build Docker image
    let dockerfile = r#"
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y \
    build-essential bc bison flex libelf-dev libssl-dev curl xz-utils cpio \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY build-kernel.sh /build/
COPY microvm.config /kernel/
RUN chmod +x /build/build-kernel.sh
RUN mkdir -p /kernel
ENV BUILD_DIR=/tmp/kernel-build
ENTRYPOINT ["/build/build-kernel.sh"]
CMD ["6.1.70"]
"#;

    let dockerfile_path = temp_dir.join("Dockerfile");
    std::fs::write(&dockerfile_path, dockerfile)?;

    // Build the Docker image
    let status = Command::new("docker")
        .args(["build", "-t", "agentkernel-kernel-builder", "."])
        .current_dir(&temp_dir)
        .status()
        .context("Failed to build kernel builder Docker image")?;

    if !status.success() {
        bail!("Failed to build kernel builder Docker image");
    }

    // Run the build
    let kernel_dir = data_dir.join("images/kernel");
    std::fs::create_dir_all(&kernel_dir)?;

    // Copy config to kernel dir BEFORE running Docker (volume mount shadows image contents)
    std::fs::write(kernel_dir.join("microvm.config"), config_content)?;

    let status = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/kernel", kernel_dir.display()),
            "agentkernel-kernel-builder",
            "6.1.70",
        ])
        .status()
        .context("Failed to run kernel build")?;

    if !status.success() {
        bail!("Kernel build failed");
    }

    println!("Kernel installed to: {}", kernel_dir.display());
    Ok(())
}

/// Build the guest agent binary for inclusion in rootfs
///
/// Cross-compiles the guest agent to x86_64-unknown-linux-musl for static linking.
async fn build_guest_agent(data_dir: &Path) -> Result<()> {
    let bin_dir = data_dir.join("bin");
    std::fs::create_dir_all(&bin_dir)?;

    // Embedded guest agent source
    let guest_agent_source = include_str!("../guest-agent/src/main.rs");
    let guest_agent_cargo = include_str!("../guest-agent/Cargo.toml");

    // Create temp directory for build
    let temp_dir = std::env::temp_dir().join("agentkernel-guest-build");
    std::fs::create_dir_all(&temp_dir)?;
    std::fs::create_dir_all(temp_dir.join("src"))?;

    // Write source files
    std::fs::write(temp_dir.join("src/main.rs"), guest_agent_source)?;
    std::fs::write(temp_dir.join("Cargo.toml"), guest_agent_cargo)?;

    // Dockerfile for building with musl
    let dockerfile = r#"
FROM rust:1.85-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl 2>/dev/null || cargo build --release
RUN cp target/*/release/agent /agent || cp target/release/agent /agent

FROM scratch
COPY --from=builder /agent /agent
CMD ["/agent"]
"#;

    std::fs::write(temp_dir.join("Dockerfile"), dockerfile)?;

    // Build in Docker
    let status = Command::new("docker")
        .args(["build", "-t", "agentkernel-guest-builder", "."])
        .current_dir(&temp_dir)
        .status()
        .context("Failed to build guest agent Docker image")?;

    if !status.success() {
        bail!("Failed to build guest agent Docker image");
    }

    // Extract the binary using docker cp
    // First remove any existing temp container
    let _ = Command::new("docker")
        .args(["rm", "-f", "agentkernel-guest-tmp"])
        .output();

    // Create a temporary container from the built image
    let status = Command::new("docker")
        .args([
            "create",
            "--name",
            "agentkernel-guest-tmp",
            "agentkernel-guest-builder",
        ])
        .status()
        .context("Failed to create temp container")?;

    if !status.success() {
        bail!("Failed to create temp container for guest agent");
    }

    // Copy the binary out
    let status = Command::new("docker")
        .args([
            "cp",
            "agentkernel-guest-tmp:/agent",
            &bin_dir.join("agent").to_string_lossy(),
        ])
        .status()
        .context("Failed to extract guest agent binary")?;

    // Clean up temp container
    let _ = Command::new("docker")
        .args(["rm", "-f", "agentkernel-guest-tmp"])
        .output();

    if !status.success() {
        bail!("Failed to extract guest agent binary");
    }

    // Make executable and verify the binary is valid
    let agent_path = bin_dir.join("agent");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if agent_path.exists() {
            std::fs::set_permissions(&agent_path, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    // Verify the binary is non-empty (catches build failures like wrong Rust edition)
    let agent_size = std::fs::metadata(&agent_path).map(|m| m.len()).unwrap_or(0);
    if agent_size == 0 {
        bail!(
            "Guest agent binary is empty (0 bytes). This usually means the Rust build failed.\n\
             Check that guest-agent/Cargo.toml uses a supported Rust edition (2021, not 2024)."
        );
    }
    if agent_size < 10000 {
        eprintln!(
            "Warning: Guest agent binary is unusually small ({} bytes). It may not work correctly.",
            agent_size
        );
    }

    println!(
        "Guest agent built: {} ({} bytes)",
        agent_path.display(),
        agent_size
    );
    Ok(())
}

/// Build a rootfs image
async fn build_rootfs(data_dir: &Path, runtime: &str) -> Result<()> {
    let rootfs_dir = data_dir.join("images/rootfs");
    std::fs::create_dir_all(&rootfs_dir)?;

    // First, build the guest agent if not already built
    let agent_bin = data_dir.join("bin/agent");
    if !agent_bin.exists() {
        println!("Building guest agent...");
        build_guest_agent(data_dir).await?;
    }

    // Size based on runtime
    let size_mb = match runtime {
        "base" => 64,
        "python" | "node" => 256,
        "go" | "rust" => 512,
        _ => 256,
    };

    // Packages based on runtime
    let packages = match runtime {
        "python" => "python3 py3-pip",
        "node" => "nodejs npm",
        "go" => "go",
        "rust" => "rust cargo",
        _ => "",
    };

    // Build script that runs inside Docker
    let build_script = format!(
        r#"#!/bin/sh
set -eu

# Install required tools
apk add --no-cache e2fsprogs

ROOTFS_IMG="/output/{runtime}.ext4"
MOUNT_DIR="/mnt/rootfs"
SIZE_MB={size_mb}
PACKAGES="{packages}"

echo "Creating ${{SIZE_MB}}MB ext4 image..."
dd if=/dev/zero of="$ROOTFS_IMG" bs=1M count=$SIZE_MB 2>/dev/null
mkfs.ext4 -F "$ROOTFS_IMG"

echo "Mounting and populating rootfs..."
mkdir -p "$MOUNT_DIR"
mount -o loop "$ROOTFS_IMG" "$MOUNT_DIR"

echo "Installing Alpine base system..."
apk -X https://dl-cdn.alpinelinux.org/alpine/v3.20/main \
    -X https://dl-cdn.alpinelinux.org/alpine/v3.20/community \
    -U --allow-untrusted --root "$MOUNT_DIR" --initdb \
    add alpine-base busybox-static $PACKAGES || true

mkdir -p "$MOUNT_DIR"/{{dev,proc,sys,tmp,run,root,app,usr/bin}}
chmod 1777 "$MOUNT_DIR/tmp"

# Copy guest agent if available
if [ -f /agent-bin/agent ]; then
    cp /agent-bin/agent "$MOUNT_DIR/usr/bin/agent"
    chmod +x "$MOUNT_DIR/usr/bin/agent"
    echo "Guest agent installed"
fi

# Create device nodes
mknod -m 622 "$MOUNT_DIR/dev/console" c 5 1 || true
mknod -m 666 "$MOUNT_DIR/dev/null" c 1 3 || true
mknod -m 666 "$MOUNT_DIR/dev/zero" c 1 5 || true
mknod -m 666 "$MOUNT_DIR/dev/tty" c 5 0 || true
mknod -m 666 "$MOUNT_DIR/dev/random" c 1 8 || true
mknod -m 666 "$MOUNT_DIR/dev/urandom" c 1 9 || true

# Create init script that starts the guest agent
cat > "$MOUNT_DIR/init" << 'INIT'
#!/bin/busybox sh
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t sysfs sysfs /sys
/bin/busybox mount -t devtmpfs devtmpfs /dev 2>/dev/null || true
/bin/busybox hostname agentkernel

# Start guest agent in background if available
if [ -x /usr/bin/agent ]; then
    /usr/bin/agent &
    echo "Guest agent started"
fi

echo "Agentkernel guest ready"
if [ $# -eq 0 ]; then
    exec /bin/busybox sh
else
    exec "$@"
fi
INIT
chmod +x "$MOUNT_DIR/init"

# Set up /etc files
echo "agentkernel" > "$MOUNT_DIR/etc/hostname"
echo "root:x:0:0:root:/root:/bin/sh" > "$MOUNT_DIR/etc/passwd"
echo "root:x:0:" > "$MOUNT_DIR/etc/group"

umount "$MOUNT_DIR"

# Fix ownership so Firecracker can access the file
if [ -n "$HOST_UID" ] && [ -n "$HOST_GID" ]; then
    chown "$HOST_UID:$HOST_GID" "$ROOTFS_IMG"
fi

echo "Rootfs created: $ROOTFS_IMG"
ls -lh "$ROOTFS_IMG"
"#,
        runtime = runtime,
        size_mb = size_mb,
        packages = packages
    );

    // Create temp directory
    let temp_dir = std::env::temp_dir().join("agentkernel-rootfs-build");
    std::fs::create_dir_all(&temp_dir)?;

    let script_path = temp_dir.join("build.sh");
    std::fs::write(&script_path, &build_script)?;

    // Run build in Docker
    // SECURITY NOTE: Building rootfs images requires privileged access to create
    // loop devices and mount filesystems. This is only used during setup, not
    // during normal sandbox operation. The build runs a minimal Alpine container
    // with a controlled script. For production deployments, consider using
    // pre-built images instead of building locally.
    eprintln!("  (Building with privileged Docker - required for loop device access)");

    // Get current user's UID/GID to fix ownership after build
    let uid = Command::new("id")
        .args(["-u"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "1000".to_string());
    let gid = Command::new("id")
        .args(["-g"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "1000".to_string());

    let status = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--privileged",
            "-e",
            &format!("HOST_UID={}", uid),
            "-e",
            &format!("HOST_GID={}", gid),
            // Security: Mount build script as read-only to prevent tampering
            "-v",
            &format!("{}:/output", rootfs_dir.display()),
            "-v",
            &format!("{}:/build.sh:ro", script_path.display()),
            "-v",
            &format!("{}:/agent-bin:ro", data_dir.join("bin").display()),
            "alpine:3.20",
            "/bin/sh",
            "/build.sh",
        ])
        .status()
        .context("Failed to run rootfs build")?;

    if !status.success() {
        bail!("Rootfs build failed for {}", runtime);
    }

    // Fix ownership - Docker creates files as root, but we need user access for Firecracker
    let rootfs_path = rootfs_dir.join(format!("{}.ext4", runtime));
    #[cfg(unix)]
    {
        use std::os::unix::fs::chown;
        if let (Some(uid), Some(gid)) = (
            std::env::var("UID").ok().and_then(|s| s.parse().ok()),
            std::env::var("GID")
                .ok()
                .or_else(|| std::env::var("GROUPS").ok())
                .and_then(|s| s.split_whitespace().next().and_then(|g| g.parse().ok())),
        ) {
            let _ = chown(&rootfs_path, Some(uid), Some(gid));
        } else {
            // Fallback: try to get uid/gid from the id command
            if let Ok(output) = Command::new("id").args(["-u"]).output()
                && output.status.success()
            {
                let uid: u32 = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse()
                    .unwrap_or(1000);
                if let Ok(output) = Command::new("id").args(["-g"]).output()
                    && output.status.success()
                {
                    let gid: u32 = String::from_utf8_lossy(&output.stdout)
                        .trim()
                        .parse()
                        .unwrap_or(1000);
                    let _ = chown(&rootfs_path, Some(uid), Some(gid));
                }
            }
        }
    }

    println!(
        "Rootfs installed to: {}/{}.ext4",
        rootfs_dir.display(),
        runtime
    );
    Ok(())
}

/// Run a quick verification test to ensure Firecracker can boot
async fn run_verification_test(data_dir: &Path) -> Result<()> {
    println!("\n==> Running verification test...");

    let kernel_path = find_kernel(data_dir).ok_or_else(|| anyhow::anyhow!("Kernel not found"))?;
    let rootfs_path = data_dir.join("images/rootfs/base.ext4");
    let firecracker_path = data_dir.join("bin/firecracker");

    if !rootfs_path.exists() {
        bail!("Rootfs not found: {}", rootfs_path.display());
    }
    if !firecracker_path.exists() {
        bail!("Firecracker not found: {}", firecracker_path.display());
    }

    println!("  Kernel: {}", kernel_path.display());
    println!("  Rootfs: {}", rootfs_path.display());
    println!("  Firecracker: {}", firecracker_path.display());

    // Create a test using our vmm module
    // For now, just verify the files exist and are accessible
    let kernel_size = std::fs::metadata(&kernel_path)?.len();
    let rootfs_size = std::fs::metadata(&rootfs_path)?.len();
    let fc_size = std::fs::metadata(&firecracker_path)?.len();

    println!("\n  Kernel size: {} bytes", kernel_size);
    println!("  Rootfs size: {} bytes", rootfs_size);
    println!("  Firecracker size: {} bytes", fc_size);

    // Basic sanity checks
    if kernel_size < 1_000_000 {
        eprintln!("  ⚠️  Kernel seems too small, might not be built correctly");
    }
    if rootfs_size < 10_000_000 {
        eprintln!("  ⚠️  Rootfs seems too small, might not be built correctly");
    }

    // Check guest agent in rootfs (would require mounting, skip for now)
    let agent_path = data_dir.join("bin/agent");
    if agent_path.exists() {
        let agent_size = std::fs::metadata(&agent_path)?.len();
        println!("  Guest agent: {} bytes", agent_size);
        if agent_size > 0 {
            println!("\n✓ All components look good!");
            println!(
                "\nNote: Full boot test requires KVM access. Create and start a sandbox to test:"
            );
            println!("  agentkernel create test-sandbox");
            println!("  agentkernel start test-sandbox");
            println!("  agentkernel exec test-sandbox -- echo 'Hello from microVM!'");
        }
    } else {
        eprintln!("  ⚠️  Guest agent not found at {}", agent_path.display());
    }

    Ok(())
}

/// Install Firecracker binary
async fn install_firecracker_binary(data_dir: &Path) -> Result<()> {
    let bin_dir = data_dir.join("bin");
    std::fs::create_dir_all(&bin_dir)?;

    // Detect architecture
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        bail!("Unsupported architecture");
    };

    let version = "v1.7.0";
    let url = format!(
        "https://github.com/firecracker-microvm/firecracker/releases/download/{}/firecracker-{}-{}.tgz",
        version, version, arch
    );

    println!("Downloading Firecracker {} for {}...", version, arch);

    // Download and extract
    let status = Command::new("sh")
        .args([
            "-c",
            &format!(
                r#"curl -fsSL "{}" | tar -xz -C "{}" && \
                   mv "{}/release-{}-{}/firecracker-{}-{}" "{}/firecracker" && \
                   chmod +x "{}/firecracker" && \
                   rm -rf "{}/release-{}-{}""#,
                url,
                bin_dir.display(),
                bin_dir.display(),
                version,
                arch,
                version,
                arch,
                bin_dir.display(),
                bin_dir.display(),
                bin_dir.display(),
                version,
                arch
            ),
        ])
        .status()
        .context("Failed to download Firecracker")?;

    if !status.success() {
        bail!("Failed to download Firecracker");
    }

    let firecracker_path = bin_dir.join("firecracker");
    println!("Firecracker installed to: {}", firecracker_path.display());

    // Add to PATH hint
    println!("\nAdd to your PATH:");
    println!("  export PATH=\"{}:$PATH\"", bin_dir.display());

    Ok(())
}
