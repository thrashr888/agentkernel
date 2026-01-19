# Agentkernel

A Firecracker-based microVM runtime for running AI coding agents in true hardware-isolated sandboxes. Sub-125ms boot times, ~25MB images, real isolation.

## Why Firecracker?

Most AI agent sandboxes use Docker containers, which share the host kernel and have weaker isolation boundaries. Agentkernel uses **Firecracker microVMs** - the same technology that powers AWS Lambda and Fargate - to provide:

- **True hardware isolation**: Each sandbox runs in its own virtual machine with dedicated kernel
- **Fast boot times**: <125ms cold start (vs 1-5s for containers)
- **Tiny footprint**: ~4MB kernel + ~20MB rootfs per VM
- **Strong security**: Separate kernel means no container escape vulnerabilities

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Agentkernel CLI                         │
├─────────────────────────────────────────────────────────────┤
│                     VMM Manager                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              Firecracker microVM                     │   │
│  │  ┌───────────────────────────────────────────────┐  │   │
│  │  │  Minimal Linux Kernel (vmlinux, ~4MB)         │  │   │
│  │  │  ┌─────────────────────────────────────────┐  │  │   │
│  │  │  │  Rootfs (squashfs/ext4, ~20MB)          │  │  │   │
│  │  │  │  - Guest Agent (vsock listener)         │  │  │   │
│  │  │  │  - Runtime (Python/Node/Go/Rust)        │  │  │   │
│  │  │  │  - Project files (virtio-blk mount)     │  │  │   │
│  │  │  └─────────────────────────────────────────┘  │  │   │
│  │  └───────────────────────────────────────────────┘  │   │
│  └─────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────┤
│  Linux: KVM (/dev/kvm)  │  macOS: Docker + KVM layer       │
└─────────────────────────────────────────────────────────────┘
```

### Platform Support

| Platform | Virtualization | Status |
|----------|---------------|--------|
| Linux (x86_64) | Native KVM | Primary target |
| Linux (aarch64) | Native KVM | Supported |
| macOS (Apple Silicon) | Docker Desktop + KVM | Planned |
| macOS (Intel) | Docker Desktop + KVM | Planned |

On macOS, we run Firecracker inside a lightweight Linux VM (Docker Desktop's VM) that provides KVM. This adds minimal overhead while maintaining the same security model.

## Quick Start

```bash
# Build from source
cargo build --release

# Build the kernel (requires Docker on macOS)
cd images/build
docker build -t agentkernel-kernel-builder -f Dockerfile.kernel-builder .
docker run --rm -v "$(pwd)/../kernel:/kernel" agentkernel-kernel-builder 6.1.70

# Create and run a microVM (Linux only, requires /dev/kvm)
./target/release/agentkernel create my-project --agent claude
./target/release/agentkernel start my-project
./target/release/agentkernel exec my-project echo "hello from microVM"
```

## Building Images

### Kernel

The microVM kernel is a minimal Linux build (~4MB) optimized for Firecracker:

```bash
cd images/build

# Build via Docker (works on any platform)
docker build -t agentkernel-kernel-builder -f Dockerfile.kernel-builder .
docker run --rm -v "$(pwd)/../kernel:/kernel" agentkernel-kernel-builder 6.1.70

# Output: images/kernel/vmlinux-6.1.70-agentkernel
```

Kernel features enabled:
- virtio (blk, net, vsock) for device communication
- ext4, squashfs, overlayfs for filesystem
- PVH entry point for fast boot
- Minimal footprint (no modules, USB, sound, graphics)

### Rootfs Images

Pre-built rootfs images for common stacks (coming soon):

| Image | Size | Contents |
|-------|------|----------|
| `base.ext4` | ~20MB | Alpine Linux, guest agent |
| `python.ext4` | ~50MB | Python 3.12, pip, common libs |
| `node.ext4` | ~40MB | Node.js 20 LTS, npm |
| `rust.ext4` | ~100MB | Rust toolchain |

## Configuration

```toml
# agentkernel.toml
[sandbox]
name = "my-project"
rootfs = "python"    # base, python, node, rust, or path to custom image

[agent]
preferred = "claude"  # claude, gemini, codex, opencode

[resources]
vcpus = 2
memory_mb = 512      # MicroVMs are memory-efficient

[network]
vsock_cid = 3        # Auto-assigned if not specified
```

## Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Boot time | <125ms | From VM create to guest agent ready |
| Shutdown time | <50ms | Graceful shutdown via vsock |
| Memory overhead | <10MB | Per-VM host memory usage |
| 100 VM test | <30s | Parallel boot, exec, shutdown |

Run the stress test:
```bash
cargo test --test stress_test -- --nocapture --ignored
```

## Development Status

**In Progress:**
- [x] Minimal kernel configuration
- [x] Kernel build infrastructure (Docker-based)
- [x] Stress test framework
- [ ] Firecracker API client
- [ ] Guest agent (vsock)
- [ ] Rootfs build system
- [ ] macOS Docker-KVM layer

See [plan/firecracker-pivot.md](plan/firecracker-pivot.md) for the full implementation plan.

## Security

MicroVM isolation provides:
- **Separate kernel**: Guest cannot exploit host kernel vulnerabilities
- **Memory isolation**: Hardware-enforced via KVM/VT-x
- **No shared namespaces**: Each VM has its own PID/network/mount space
- **Minimal attack surface**: Firecracker has <50k lines of Rust code

## Inspiration

- [Firecracker](https://firecracker-microvm.github.io/) - Secure microVM technology from AWS
- [Ramp's Background Agent](https://builders.ramp.com/post/why-we-built-our-background-agent) - Cloud sandbox architecture for AI agents
- [Modal](https://modal.com/) - Serverless sandbox infrastructure
- [Fly.io](https://fly.io/) - Firecracker-based app platform

## License

MIT
