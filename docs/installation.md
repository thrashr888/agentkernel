
# Installation

## Prerequisites

- **Linux**: KVM-enabled host (most cloud VMs, bare metal)
- **macOS**: Docker Desktop or Apple Containers (macOS 26+)
- **Windows**: WSL2 with Docker (untested)

## Quick Install

```bash
curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
```

This installs the `agentkernel` binary to `~/.local/bin/`.

## Manual Install

### From Source

```bash
git clone https://github.com/thrashr888/agentkernel
cd agentkernel
cargo build --release
cp target/release/agentkernel ~/.local/bin/
```

### From Releases

Download the latest release from [GitHub Releases](https://github.com/thrashr888/agentkernel/releases).

## Setup

After installation, run setup to configure your backend:

```bash
agentkernel setup
```

This will:
1. Detect available backends (Firecracker, Docker, Podman, Apple Containers)
2. Download required images
3. Configure default settings

## Backend-Specific Setup

### Linux (Firecracker)

Requires KVM access:

```bash
# Add user to kvm group
sudo usermod -aG kvm $USER

# Verify KVM access
ls -la /dev/kvm
```

### macOS (Docker Desktop)

Install [Docker Desktop](https://www.docker.com/products/docker-desktop/) and ensure it's running.

### macOS 26+ (Apple Containers)

Apple Containers is built-in to macOS 26+. No additional setup required.

## Verify Installation

```bash
agentkernel --version
agentkernel run echo "Hello from sandbox!"
```
