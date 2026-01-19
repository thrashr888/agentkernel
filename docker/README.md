# Docker-based KVM Host for macOS

Run Firecracker microVMs on macOS using Docker Desktop's nested virtualization.

## Prerequisites

- Docker Desktop for Mac with Virtualization.framework enabled
- For best compatibility, use Apple Silicon (M1/M2/M3) Macs

## Quick Start

```bash
# Build the KVM host image
docker build -t agentkernel-kvm-host -f docker/Dockerfile.kvm-host .

# Run the KVM host (interactive)
docker run --rm -it \
  --privileged \
  --device /dev/kvm \
  -v "$(pwd)/images:/opt/agentkernel/images" \
  agentkernel-kvm-host bash

# Inside the container, you can run Firecracker
firecracker --api-sock /tmp/firecracker.sock
```

## Volume Mounts

Mount the following volumes for full functionality:

| Host Path | Container Path | Purpose |
|-----------|----------------|---------|
| `./images` | `/opt/agentkernel/images` | Kernel and rootfs images |
| `/var/run/docker.sock` | `/var/run/docker.sock` | Docker-in-Docker (optional) |

## Docker Desktop Configuration

1. Open Docker Desktop Settings
2. Go to "General" tab
3. Ensure "Use Virtualization.framework" is enabled (macOS 12.5+)
4. For Intel Macs, you may need to enable "Use Rosetta for x86_64/amd64 emulation on Apple Silicon"

## Limitations

- Nested virtualization performance is reduced compared to native KVM
- Not all KVM features may be available
- Network bridging may require additional configuration

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                     macOS Host                          │
│  ┌──────────────────────────────────────────────────┐  │
│  │              Docker Desktop                       │  │
│  │  ┌────────────────────────────────────────────┐  │  │
│  │  │         Linux VM (Virtualization.fw)       │  │  │
│  │  │  ┌──────────────────────────────────────┐  │  │  │
│  │  │  │    agentkernel-kvm-host container    │  │  │  │
│  │  │  │  ┌────────────────────────────────┐  │  │  │  │
│  │  │  │  │        Firecracker VMM         │  │  │  │  │
│  │  │  │  │  ┌──────────────────────────┐  │  │  │  │  │
│  │  │  │  │  │       microVM            │  │  │  │  │  │
│  │  │  │  │  │   (guest kernel + fs)    │  │  │  │  │  │
│  │  │  │  │  └──────────────────────────┘  │  │  │  │  │
│  │  │  │  └────────────────────────────────┘  │  │  │  │
│  │  │  └──────────────────────────────────────┘  │  │  │
│  │  └────────────────────────────────────────────┘  │  │
│  └──────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```
