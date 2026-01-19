# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**agentkernel** is a Firecracker-based microVM runtime for running AI coding agents (Claude Code, Gemini CLI, Codex, OpenCode) in true hardware-isolated sandboxes. It provides per-project isolation with sub-125ms boot times, ~25MB images, and real kernel-level isolation via KVM.

**Key difference from containers**: Each sandbox runs in its own virtual machine with a dedicated Linux kernel, not a shared kernel like Docker. This provides stronger isolation guarantees.

## Build Commands

```bash
# Build
cargo build

# Build release
cargo build --release

# Run tests
cargo test

# Run stress test (100 VMs, requires Firecracker implementation)
cargo test --test stress_test -- --nocapture --ignored

# Quality gates (run before commits)
cargo fmt -- --check && cargo clippy -- -D warnings && cargo test

# Run the CLI
cargo run -- <command>
```

## Building VM Images

```bash
# Build kernel (via Docker, works on macOS)
cd images/build
docker build -t agentkernel-kernel-builder -f Dockerfile.kernel-builder .
docker run --rm -v "$(pwd)/../kernel:/kernel" agentkernel-kernel-builder 6.1.70

# Output: images/kernel/vmlinux-6.1.70-agentkernel (~4MB)
```

## Architecture

```
src/
├── main.rs           # CLI entry point (clap-based), command dispatch
├── config.rs         # Config parsing for agentkernel.toml
├── permissions.rs    # Security profiles and permission management
├── agents.rs         # Multi-agent support (Claude, Gemini, Codex, OpenCode)
├── http_api.rs       # HTTP REST API server
├── mcp.rs            # MCP server for Claude Code integration
├── docker_backend.rs # Docker/Podman container backend
├── vmm.rs            # Virtual machine manager (abstracts backends)
├── languages.rs      # Language/runtime detection
└── setup.rs          # Setup and installation management

images/
├── kernel/
│   ├── microvm.config              # Minimal kernel config for Firecracker
│   └── vmlinux-*-agentkernel       # Built kernel (after build)
├── rootfs/                          # Rootfs images (TODO)
└── build/
    ├── build-kernel.sh             # Kernel build script
    └── Dockerfile.kernel-builder    # Docker build environment

tests/
└── stress_test.rs                   # 100 VM parallel stress test

plan/
└── firecracker-pivot.md            # Full architecture plan
```

### Key Components (In Development)

- **Firecracker VMM**: Direct API integration for microVM lifecycle
- **Guest Agent**: vsock-based daemon inside VM for command execution
- **Kernel**: Minimal Linux build (~4MB) with virtio, vsock, ext4
- **Rootfs**: Alpine-based images with runtime stacks (Python, Node, Rust)

### Platform Strategy

| Platform | How it works |
|----------|--------------|
| Linux | Direct Firecracker + /dev/kvm |
| macOS | Docker Desktop provides KVM-capable Linux VM, Firecracker runs inside |

### Configuration Schema (agentkernel.toml)

```toml
[sandbox]
name = "my-project"
base_image = "python:3.12-alpine"  # Or use runtime shorthand

[agent]
preferred = "claude"      # claude, gemini, codex, opencode

[resources]
vcpus = 2
memory_mb = 512

[security]
profile = "restrictive"   # permissive, moderate, restrictive
network = false           # Override: disable network
mount_cwd = false         # Override: mount current directory

[network]
vsock_cid = 3             # Auto-assigned if not specified
```

### Security Profiles

| Profile | Network | Mount CWD | Mount Home | Pass Env | Read-only |
|---------|---------|-----------|------------|----------|-----------|
| permissive | Yes | Yes | Yes | Yes | No |
| moderate | Yes | No | No | No | No |
| restrictive | No | No | No | No | Yes |

## Key Dependencies

- `clap` with derive - CLI argument parsing
- `tokio` - async runtime
- `anyhow` - error handling
- `serde/toml` - config parsing
- (Planned) Firecracker API client via REST/Unix socket

## Performance Targets

| Metric | Target |
|--------|--------|
| Boot time | <125ms |
| Shutdown | <50ms |
| Memory overhead | <10MB per VM |
| 100 VM test | <30s total |

## Security Model

MicroVM isolation provides:
- **Separate kernel**: Each VM runs its own Linux kernel
- **Hardware isolation**: KVM/VT-x enforced memory boundaries
- **Minimal attack surface**: Firecracker is ~50k lines of Rust
- **No container escapes**: Not sharing host kernel
- **Security profiles**: `restrictive` (default in examples), `moderate`, `permissive`
- **Network control**: `--no-network` flag or config override

## Beads Issue Tracking

```bash
bd ready              # Show unblocked work
bd show <id>          # View issue details
bd update <id> --status in_progress
bd close <id>
bd sync               # Sync with git (run at session end)
```
