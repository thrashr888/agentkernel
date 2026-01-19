# Firecracker Pivot: True Microkernel Architecture

## Executive Summary

Pivot agentkernel from Docker containers to Firecracker microVMs for true hardware-level isolation, sub-125ms startup times, and minimal resource footprint. Use Docker as a Linux VM layer on macOS/Windows to provide KVM access.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      agentkernel CLI                            │
├─────────────────────────────────────────────────────────────────┤
│                      Platform Abstraction                        │
├────────────────────────┬────────────────────────────────────────┤
│   Linux (native KVM)   │   macOS/Windows (Docker VM layer)      │
│                        │                                         │
│   ┌──────────────┐     │   ┌─────────────────────────────────┐  │
│   │  Firecracker │     │   │  Docker container (Linux VM)    │  │
│   │     VMM      │     │   │  ┌───────────────────────────┐  │  │
│   └──────┬───────┘     │   │  │      Firecracker VMM      │  │  │
│          │             │   │  └───────────┬───────────────┘  │  │
│   ┌──────▼───────┐     │   │              │                  │  │
│   │   microVM    │     │   │  ┌───────────▼───────────────┐  │  │
│   │  (KVM/guest) │     │   │  │        microVM            │  │  │
│   └──────────────┘     │   │  │       (nested KVM)        │  │  │
│                        │   │  └───────────────────────────┘  │  │
│                        │   └─────────────────────────────────┘  │
└────────────────────────┴────────────────────────────────────────┘
```

## Key Components

### 1. Firecracker VMM Integration

Use Firecracker (or Cloud Hypervisor as fallback) as the microVM backend:

- **Firecracker**: Amazon's microVM, ~1MB binary, less than 125ms boot, ~5MB memory overhead
- **Cloud Hypervisor**: Intel/Microsoft alternative, more features, similar performance
- **QEMU microvm**: Fallback for broader compatibility

### 2. Platform Detection and Abstraction

```rust
enum Platform {
    LinuxNative,      // Direct KVM access
    LinuxContainer,   // Inside Docker (nested virt)
    MacOSDocker,      // Docker Desktop with Linux VM
    WindowsDocker,    // Docker Desktop with WSL2/Hyper-V
    Unsupported,
}

trait VMMBackend {
    async fn create_vm(&self, config: &VMConfig) -> Result<VM>;
    async fn start_vm(&self, vm: &VM) -> Result<()>;
    async fn stop_vm(&self, vm: &VM) -> Result<()>;
    async fn run_command(&self, vm: &VM, cmd: &[String]) -> Result<Output>;
}
```

### 3. Minimal Guest Images

Pre-built kernel + rootfs images optimized for AI agent workloads:

```
images/
├── kernel/
│   └── vmlinux-5.10-minimal    # ~4MB stripped kernel
├── rootfs/
│   ├── base.ext4               # ~20MB minimal Alpine
│   ├── python.ext4             # +Python runtime (~50MB)
│   ├── node.ext4               # +Node.js runtime (~40MB)
│   └── rust.ext4               # +Rust toolchain (~100MB)
└── overlays/
    └── agent-{claude,gemini,codex,opencode}.ext4
```

### 4. Docker VM Layer (macOS/Windows)

For non-Linux hosts, run a minimal Linux VM via Docker that provides KVM:

```dockerfile
# Dockerfile.kvm-host
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y \
    qemu-kvm \
    firecracker \
    && rm -rf /var/lib/apt/lists/*
# Enable nested virtualization
```

Docker Desktop on macOS uses Apple Virtualization.framework which supports nested virt.

## Implementation Phases

### Phase 1: Firecracker Integration (Linux)

**Goal**: Get microVMs working on Linux with direct KVM

1. Add Firecracker API client (HTTP over Unix socket)
2. Create VM lifecycle management (create, start, stop, destroy)
3. Implement virtio-vsock for host-to-guest communication
4. Build minimal kernel image (vmlinux)
5. Build minimal rootfs with busybox/Alpine base
6. Implement agentkernel create/start/stop/attach for microVMs

**Deliverables**:
- `src/vmm/firecracker.rs` - Firecracker API client
- `src/vmm/mod.rs` - VMM trait and abstractions
- `src/images/mod.rs` - Image management
- `images/` - Pre-built kernel and rootfs

### Phase 2: Guest Agent and Communication

**Goal**: Rich interaction between host and guest VM

1. Build guest agent binary (runs inside microVM)
2. Implement vsock-based RPC protocol
3. File sync between host and guest (virtio-fs or rsync over vsock)
4. Port forwarding for network access
5. PTY multiplexing for interactive shells

**Deliverables**:
- `src/agent/` - Guest agent (separate binary, cross-compiled)
- `src/vsock/` - vsock communication layer
- `src/sync/` - File synchronization

### Phase 3: Platform Abstraction (macOS/Windows)

**Goal**: Run on macOS/Windows via Docker VM layer

1. Detect platform and available virtualization
2. Build Docker image with Firecracker + KVM support
3. Implement Docker-based VMM backend
4. Handle nested virtualization setup
5. Network bridging between Docker and microVM

**Deliverables**:
- `src/platform/` - Platform detection
- `docker/Dockerfile.kvm-host` - Linux VM image
- `src/vmm/docker_kvm.rs` - Docker-wrapped Firecracker

### Phase 4: Agent Runtime Images

**Goal**: Pre-built images with AI agent tooling

1. Create base image build system
2. Build Python/Node/Rust runtime images
3. Package Claude Code, Gemini CLI, Codex, OpenCode
4. Implement image pulling/caching
5. Support custom image building

**Deliverables**:
- `images/build/` - Image build scripts
- Image registry or bundled images
- `src/images/registry.rs` - Image management

### Phase 5: Production Hardening

**Goal**: Security, performance, reliability

1. Jailer integration (Firecracker's security sandbox)
2. Resource limits and accounting
3. Snapshot/restore for instant starts
4. Image deduplication and CoW
5. Metrics and observability

## Technical Decisions

### Firecracker vs Cloud Hypervisor vs QEMU

| Feature | Firecracker | Cloud Hypervisor | QEMU microvm |
|---------|-------------|------------------|--------------|
| Boot time | <125ms | <200ms | ~500ms |
| Memory | ~5MB | ~10MB | ~30MB |
| Binary size | ~1MB | ~3MB | ~10MB |
| GPU passthrough | No | Yes | Yes |
| Nested virt | Limited | Yes | Yes |
| macOS support | No (Linux only) | No | Via HVF |

**Recommendation**: Start with Firecracker for Linux, use Cloud Hypervisor as fallback for nested virt scenarios.

### Communication: vsock vs virtio-net

- **vsock**: Direct host-to-guest socket, no network stack overhead, preferred
- **virtio-net**: Standard networking, needed for internet access from guest

Use vsock for control plane, virtio-net for data plane.

### Rootfs: ext4 vs squashfs vs erofs

- **ext4**: Read-write, standard, larger
- **squashfs**: Read-only, compressed, good for base images
- **erofs**: Read-only, faster than squashfs, newer

**Recommendation**: squashfs for base images, ext4 overlay for writes.

## File Structure (Proposed)

```
src/
├── main.rs
├── cli.rs              # CLI argument parsing
├── config.rs           # agentkernel.toml parsing
├── platform/
│   ├── mod.rs          # Platform detection
│   ├── linux.rs        # Linux-specific
│   └── docker.rs       # Docker VM layer
├── vmm/
│   ├── mod.rs          # VMM trait
│   ├── firecracker.rs  # Firecracker backend
│   ├── cloud_hypervisor.rs  # CH backend (optional)
│   └── docker_kvm.rs   # Docker-wrapped VMM
├── images/
│   ├── mod.rs          # Image management
│   ├── registry.rs     # Image pulling/caching
│   └── builder.rs      # Image building
├── guest/
│   ├── mod.rs          # Guest agent protocol
│   └── vsock.rs        # vsock communication
└── sync/
    └── mod.rs          # File synchronization

images/
├── kernel/
│   └── vmlinux-5.10
├── rootfs/
│   ├── base.squashfs
│   └── python.squashfs
└── build/
    ├── kernel.sh
    └── rootfs.sh
```

## Dependencies

```toml
[dependencies]
# Existing
anyhow = "1.0"
clap = { version = "4.0", features = ["derive"] }
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.0", features = ["full"] }

# New for Firecracker
hyper = { version = "1.0", features = ["client", "http1"] }
hyper-util = "0.1"
tokio-vsock = "0.5"      # vsock communication
nix = "0.29"             # Unix APIs (KVM, etc.)

# Optional: Docker fallback
bollard = "0.18"         # Keep for Docker VM layer
```

## Open Questions

1. **Image distribution**: Bundle images in binary, or download on first run?
2. **Nested virt performance**: How much overhead on macOS Docker?
3. **GPU passthrough**: Needed for some AI workloads?
4. **Snapshot support**: Worth the complexity for faster restarts?
5. **Multi-VM**: Run multiple agents in parallel?

## Success Metrics

- VM boot time under 125ms on Linux
- VM boot time under 500ms on macOS (via Docker)
- Base image size under 50MB
- Memory overhead under 10MB per VM
- Single static binary (no runtime dependencies on Linux)
- Works on: Linux x86_64, Linux aarch64, macOS (via Docker)

## Next Steps

1. Create beads for Phase 1 tasks
2. Set up kernel/rootfs build pipeline
3. Implement basic Firecracker API client
4. Get first microVM booting on Linux
5. Iterate from there

## References

- Firecracker GitHub: github.com/firecracker-microvm/firecracker
- Firecracker Design docs in the repo
- Cloud Hypervisor: github.com/cloud-hypervisor/cloud-hypervisor
- Linux microvm machine type (QEMU docs)
- virtio-vsock specification
