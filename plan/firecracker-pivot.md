# Firecracker Pivot: True Microkernel Architecture

## Executive Summary

Pivot agentkernel from Docker containers to Firecracker microVMs for true hardware-level isolation, sub-125ms startup times, and minimal resource footprint. Use Docker/Podman as a container fallback on macOS/Windows when KVM is unavailable.

## Current Status

### Completed (P1 Features)
- [x] Firecracker API client (native Rust HTTP over Unix socket)
- [x] VM lifecycle management (create, start, stop, remove)
- [x] Minimal kernel build (~4MB vmlinux-6.1.70)
- [x] Alpine-based rootfs images (base: 64MB, python: 256MB, node: 256MB)
- [x] Container backend fallback (Docker/Podman)
- [x] CLI commands (create, start, stop, attach, exec, run, list)
- [x] Security profiles (permissive, moderate, restrictive)
- [x] Multi-agent support (Claude, Gemini, Codex, OpenCode)
- [x] MCP server integration
- [x] HTTP API server

### In Progress (P1/P2)
- [ ] vsock communication layer (host-to-guest)
- [ ] Guest agent binary
- [ ] Docker-based KVM host for macOS

### Pending (P2+)
- [ ] File sync (virtio-fs or rsync over vsock)
- [ ] Snapshot/restore for instant starts
- [ ] Image deduplication
- [ ] Jailer integration (Firecracker security sandbox)

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                      agentkernel CLI                            │
├─────────────────────────────────────────────────────────────────┤
│                      VmManager (src/vmm.rs)                     │
├────────────────────────┬────────────────────────────────────────┤
│   Linux (native KVM)   │   macOS/Windows (Container fallback)   │
│                        │                                         │
│   ┌──────────────┐     │   ┌─────────────────────────────────┐  │
│   │ Firecracker  │     │   │    Docker/Podman Container     │  │
│   │  API Client  │     │   │                                 │  │
│   └──────┬───────┘     │   │   (runs agentkernel-base:*)    │  │
│          │             │   └─────────────────────────────────┘  │
│   ┌──────▼───────┐     │                                         │
│   │   microVM    │     │   Future: Docker-wrapped Firecracker   │
│   │  (KVM/guest) │     │   for true microVM on non-Linux hosts  │
│   └──────────────┘     │                                         │
└────────────────────────┴────────────────────────────────────────┘
```

## Component Design

### 1. Firecracker API Client (`src/firecracker_client.rs`)

Native Rust HTTP client over Unix sockets using hyper:

```rust
pub struct FirecrackerClient {
    socket_path: String,
}

// API structures
pub struct BootSource { kernel_image_path, boot_args }
pub struct Drive { drive_id, path_on_host, is_root_device, is_read_only }
pub struct MachineConfig { vcpu_count, mem_size_mib }
pub struct VsockDevice { guest_cid, uds_path }

impl FirecrackerClient {
    pub async fn set_boot_source(&self, boot_source: &BootSource) -> Result<()>;
    pub async fn set_drive(&self, drive_id: &str, drive: &Drive) -> Result<()>;
    pub async fn set_machine_config(&self, config: &MachineConfig) -> Result<()>;
    pub async fn set_vsock(&self, vsock: &VsockDevice) -> Result<()>;
    pub async fn start_instance(&self) -> Result<()>;
    pub async fn send_ctrl_alt_del(&self) -> Result<()>;
}
```

### 2. Guest Agent Design (TODO: `src/guest_agent/`)

A minimal binary that runs inside the microVM and handles commands from the host.

```
┌────────────────────────────────────────────┐
│                  Host                       │
│                                             │
│   agentkernel exec test-sandbox ls -la     │
│         │                                   │
│         ▼                                   │
│   ┌─────────────┐                          │
│   │ vsock conn  │ (CID: 3, Port: 52000)    │
│   │ to guest    │                          │
│   └──────┬──────┘                          │
└──────────│─────────────────────────────────┘
           │
    ═══════│═══════  VM Boundary  ═══════════
           │
┌──────────│─────────────────────────────────┐
│   ┌──────▼──────┐                          │
│   │ Guest Agent │                          │
│   └──────┬──────┘                          │
│          │                                  │
│          ▼                                  │
│   exec("ls", ["-la"])                      │
│          │                                  │
│          ▼                                  │
│   stdout/stderr → vsock response           │
│                                             │
│             microVM Guest                   │
└────────────────────────────────────────────┘
```

#### Guest Agent Protocol (JSON over vsock)

```json
// Request (host → guest)
{
  "id": "uuid",
  "type": "exec",
  "command": ["ls", "-la"],
  "cwd": "/app",
  "env": {"PATH": "/usr/bin"}
}

// Response (guest → host)
{
  "id": "uuid",
  "exit_code": 0,
  "stdout": "...",
  "stderr": ""
}

// Request types:
// - exec: Run command, return output
// - shell: Start PTY session (streaming)
// - ping: Health check
// - shutdown: Graceful shutdown
```

#### Guest Agent Binary

Compile as a static musl binary for portability:

```rust
// guest-agent/src/main.rs
use tokio_vsock::{VsockListener, VsockStream};

const VSOCK_PORT: u32 = 52000;
const HOST_CID: u32 = 2;  // Host is always CID 2

#[tokio::main]
async fn main() -> Result<()> {
    let listener = VsockListener::bind(VMADDR_CID_ANY, VSOCK_PORT)?;

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(handle_connection(stream));
    }
}

async fn handle_connection(stream: VsockStream) -> Result<()> {
    // Read JSON request
    // Execute command
    // Send JSON response
}
```

Build:
```bash
# Cross-compile for musl (static binary)
cargo build --target x86_64-unknown-linux-musl --release
# or for ARM64
cargo build --target aarch64-unknown-linux-musl --release
```

### 3. vsock Communication Layer (TODO: `src/vsock.rs`)

```rust
use tokio_vsock::VsockStream;

/// Connect to guest agent via vsock
pub async fn connect_to_guest(cid: u32, port: u32) -> Result<VsockStream> {
    VsockStream::connect(cid, port).await
}

/// Send a command to the guest agent
pub async fn exec_in_guest(cid: u32, cmd: &[String]) -> Result<ExecResult> {
    let stream = connect_to_guest(cid, AGENT_PORT).await?;

    let request = ExecRequest {
        id: Uuid::new_v4().to_string(),
        type_: "exec".to_string(),
        command: cmd.to_vec(),
        cwd: None,
        env: None,
    };

    // Send request
    let request_bytes = serde_json::to_vec(&request)?;
    stream.write_all(&(request_bytes.len() as u32).to_le_bytes()).await?;
    stream.write_all(&request_bytes).await?;

    // Read response
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes) as usize;

    let mut response_bytes = vec![0u8; len];
    stream.read_exact(&mut response_bytes).await?;

    Ok(serde_json::from_slice(&response_bytes)?)
}
```

### 4. Rootfs Structure

```
/
├── bin/              # busybox symlinks
├── dev/              # device nodes (created at boot)
├── etc/
│   ├── hostname      # "agentkernel"
│   ├── passwd        # root:x:0:0:...
│   └── group         # root:x:0:
├── init              # Boot script (mounts /proc, /sys, starts agent)
├── proc/             # procfs mount point
├── sys/              # sysfs mount point
├── tmp/              # tmpfs
├── app/              # Working directory for user code
├── usr/
│   └── bin/
│       └── agent     # Guest agent binary
└── root/             # Root home directory
```

### 5. Security Model

| Layer | Protection |
|-------|------------|
| Host kernel | KVM hypervisor enforces VM isolation |
| Firecracker | Minimal VMM (~50k LoC Rust), reduced attack surface |
| Guest kernel | Separate kernel instance per sandbox |
| Guest rootfs | Read-only base + writable overlay |
| Network | No network by default (restrictive profile) |
| Filesystem | No host mounts by default |

### 6. Platform Strategy

| Platform | Backend | Notes |
|----------|---------|-------|
| Linux with KVM | Firecracker | Native, best performance |
| Linux no KVM | Docker/Podman | Container fallback |
| macOS | Docker/Podman | Container fallback (future: Docker-wrapped Firecracker) |
| Windows | Docker/Podman | Container fallback via WSL2 |

## Implementation Plan

### Phase 1: Core (COMPLETED)
- [x] Firecracker API client
- [x] VM lifecycle management
- [x] Kernel build infrastructure
- [x] Rootfs build infrastructure
- [x] CLI commands
- [x] Container fallback

### Phase 2: Guest Communication (IN PROGRESS)
- [ ] Build guest agent binary
- [ ] Implement vsock client in host
- [ ] Wire up `agentkernel exec` for Firecracker backend
- [ ] Add `agentkernel attach` for interactive shell

### Phase 3: File Sync
- [ ] virtio-fs for shared directories (requires kernel support)
- [ ] Alternative: rsync over vsock
- [ ] Mount project directory into guest

### Phase 4: macOS Native
- [ ] Docker-based KVM host image
- [ ] Nested Firecracker inside Docker
- [ ] Network bridging

### Phase 5: Hardening
- [ ] Jailer integration
- [ ] Snapshot/restore
- [ ] Resource accounting
- [ ] Image deduplication

## Performance Targets

| Metric | Target | Current |
|--------|--------|---------|
| Boot time (Linux) | <125ms | TBD |
| Boot time (macOS container) | <2s | ~1s |
| Memory overhead | <10MB | ~5MB (Firecracker) |
| Base image size | <50MB | 64MB |
| 100 VM stress test | <30s | TBD |

## Key Files

```
src/
├── main.rs               # CLI entry point
├── vmm.rs                # VM manager (backend selection)
├── firecracker_client.rs # Firecracker API client
├── docker_backend.rs     # Container fallback
├── config.rs             # agentkernel.toml parsing
├── setup.rs              # Installation/setup
├── permissions.rs        # Security profiles
├── languages.rs          # Runtime detection
├── agents.rs             # AI agent detection
├── mcp.rs                # MCP server
└── http_api.rs           # HTTP API server

images/
├── kernel/
│   ├── microvm.config    # Minimal kernel config
│   └── vmlinux-*         # Built kernel
├── rootfs/
│   ├── base.ext4         # Minimal Alpine (64MB)
│   ├── python.ext4       # Python runtime (256MB)
│   └── node.ext4         # Node.js runtime (256MB)
└── build/
    ├── build-kernel.sh   # Kernel build script
    └── build-rootfs.sh   # Rootfs build script
```

## References

- Firecracker: github.com/firecracker-microvm/firecracker
- tokio-vsock: docs.rs/tokio-vsock
- Alpine Linux: alpinelinux.org
- Firecracker API spec: github.com/firecracker-microvm/firecracker/blob/main/src/api_server/swagger/firecracker.yaml
