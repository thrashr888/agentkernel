---
layout: default
title: Installation
nav_order: 2
---

# Installation

## Requirements

- **macOS** (Intel or Apple Silicon) or **Linux** (x86_64)
- One of:
  - Docker Desktop (macOS/Linux)
  - Podman (Linux/macOS)
  - KVM support (Linux, for Firecracker backend)

## Quick Install

```bash
# Download and install
curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh

# Run setup to configure backends and download components
agentkernel setup
```

## Install with Cargo

```bash
cargo install agentkernel
agentkernel setup
```

## Install from Source

```bash
git clone https://github.com/thrashr888/agentkernel.git
cd agentkernel
cargo build --release
./target/release/agentkernel setup
```

## Setup Command

After installing, run `agentkernel setup` to:

1. Detect available backends (Docker, Podman, Firecracker, Apple Containers)
2. Download required kernel and rootfs images (for Firecracker)
3. Configure default settings

```bash
$ agentkernel setup

Checking system requirements...
  Docker: available (24.0.7)
  Podman: not found
  KVM: not available (macOS)
  Apple Containers: available (macOS 26+)

Selected backend: docker

Setup complete! Run 'agentkernel run echo hello' to test.
```

## Backend-Specific Setup

### Docker (Recommended for macOS)

Install [Docker Desktop](https://www.docker.com/products/docker-desktop/) and ensure it's running.

```bash
docker --version  # Verify installation
agentkernel setup
```

### Podman (Linux alternative)

```bash
# Fedora/RHEL
sudo dnf install podman

# Ubuntu/Debian
sudo apt install podman

agentkernel setup
```

### Firecracker (Linux with KVM)

Requires Linux with KVM support. Provides the fastest and most isolated sandboxes.

```bash
# Check KVM support
ls /dev/kvm

# Install Firecracker (setup does this automatically)
agentkernel setup
```

### Apple Containers (macOS 26+)

Native container support on macOS Tahoe (26+). Automatically detected by setup.

## Verify Installation

```bash
# Check version
agentkernel --version

# Run a test command
agentkernel run echo "Hello from sandbox!"

# List available backends
agentkernel setup --check
```

## Uninstall

```bash
# Remove binary
rm $(which agentkernel)

# Remove data directory (sandboxes, images, config)
rm -rf ~/.local/share/agentkernel
```
