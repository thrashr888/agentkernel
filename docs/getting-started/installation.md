
# Installation

## Prerequisites

- **Linux**: Docker/Podman for container execution, or installed Firecracker with usable `/dev/kvm` for microVMs. Cloud guests need explicit nested-virtualization support; do not assume it is available.
- **macOS**: Docker Desktop/Podman, or the separately installed Apple `container` CLI on Apple Silicon with macOS 26+.
- **Windows**: WSL2 with Docker (untested)

## Quick Install

### Homebrew (Recommended)

```bash
brew tap thrashr888/tap && brew install agentkernel
```

### Install Script

```bash
curl -fsSL https://raw.githubusercontent.com/thrashr888/agentkernel/main/install.sh | sh
```

This installs the `agentkernel` binary to `~/.local/bin/`.

## Manual Install

### From Source

Use Rust 1.89 or newer. Kubernetes, Nomad, and enterprise support are enabled by default; Hyperlight is optional.

```bash
git clone https://github.com/thrashr888/agentkernel
cd agentkernel
cargo build --release
mkdir -p ~/.local/bin
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

Install the signed `container` package using [Apple's installation instructions](https://github.com/apple/container#initial-install), then start the service:

```bash
container system start
container --version
```

AgentKernel starts the service on demand when needed, but the Apple CLI must already be installed. See [backend compatibility](../operations/backend-compatibility.md) for the version tested by this repository.

## Verify Installation

```bash
agentkernel --version
agentkernel doctor
agentkernel run echo "Hello from sandbox!"
```
