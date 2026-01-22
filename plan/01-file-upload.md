# Plan: File Upload to Sandboxed Containers

## Problem Statement

agentkernel currently has no mechanism to upload files into running sandboxes beyond bind-mounting entire directories at startup. AI coding agents frequently need to:

1. **Inject single files** - Configuration files, scripts, API credentials
2. **Stream code snippets** - Dynamic code generation that needs execution
3. **Transfer build artifacts** - Compiled binaries, dependencies, data files
4. **Provide context files** - Documents, schemas, templates the agent needs to reference

## Current State

**SandboxConfig** (`src/backend/mod.rs`):
- `mount_cwd: bool` - Mount current working directory
- `mount_home: bool` - Mount home directory (read-only)
- No file injection mechanism

**Docker Backend** - Uses `-v` flags for bind mounts at container start, no `docker cp` integration
**Firecracker Backend** - No file injection; rootfs baked into ext4 image
**Guest Agent** - Only supports `Run`, `Ping`, `Shutdown` commands

## Design Options

### Option A: vsock-Based File Transfer (Recommended for Firecracker)

Extend the guest agent protocol:
```rust
pub enum RequestType {
    Run, Ping, Shutdown,
    WriteFile { path: String, content: Vec<u8>, mode: u32 },
    ReadFile { path: String },
    Mkdir { path: String, recursive: bool },
    RemoveFile { path: String },
}
```

**Pros**: Uses existing vsock infrastructure, no kernel changes, secure
**Cons**: Serialization overhead, single-threaded transfer

### Option B: docker cp for Container Backends

Use native `docker cp` command for Docker/Podman/Apple backends.

**Pros**: Native, well-tested, handles directories recursively
**Cons**: Requires file on host first, different API than Firecracker

### Option C: Base64 via Command (Fallback)

For tiny config files (<1KB), encode and write via shell command.

## Recommended Approach: Unified Sandbox Trait Extension

```rust
#[async_trait]
pub trait Sandbox: Send + Sync {
    // Existing methods
    async fn start(&mut self, config: &SandboxConfig) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;

    // New file operations
    async fn write_file(&mut self, path: &str, content: &[u8]) -> Result<()>;
    async fn read_file(&mut self, path: &str) -> Result<Vec<u8>>;
    async fn remove_file(&mut self, path: &str) -> Result<()>;
    async fn mkdir(&mut self, path: &str, recursive: bool) -> Result<()>;
}
```

### Backend Implementations

| Backend | Implementation |
|---------|---------------|
| Docker/Podman | `docker cp` for files, temp dir staging |
| Apple Containers | `container cp` equivalent |
| Firecracker | vsock guest agent protocol extension |
| Hyperlight | WASI filesystem imports (future) |

## Implementation Phases

### Phase 1: Core Trait and Docker Backend (1-2 hours)
1. Extend Sandbox trait with file operations
2. Implement for DockerSandbox using `docker cp`
3. Add path validation (prevent traversal attacks)
4. Unit tests

### Phase 2: Firecracker Guest Agent (2-3 hours)
1. Extend AgentRequest with WriteFile, ReadFile types
2. Implement file handlers in guest agent
3. Update vsock.rs on host side
4. Rebuild guest agent binary

### Phase 3: CLI and API Integration (1-2 hours)
```bash
agentkernel cp ./local/file sandbox-name:/remote/path
agentkernel cp sandbox-name:/remote/path ./local/file
```

### Phase 4: Config File Support
```toml
[[files]]
source = "./config.json"
dest = "/app/config.json"
mode = "0644"
```

## Security Considerations

### Path Validation
```rust
pub fn validate_sandbox_path(path: &str) -> Result<()> {
    if !path.starts_with('/') { bail!("Path must be absolute"); }
    if path.contains("..") { bail!("Path traversal not allowed"); }
    let blocked = ["/proc", "/sys", "/dev", "/etc/passwd", "/etc/shadow"];
    for b in blocked {
        if path.starts_with(b) { bail!("Cannot write to system path: {}", b); }
    }
    Ok(())
}
```

### Size Limits
| Context | Max Size |
|---------|----------|
| Single file | 50 MB |
| Batch upload | 200 MB |
| Per-request | 10 MB |

## Critical Files for Implementation

1. `src/backend/mod.rs` - Add file operation methods to Sandbox trait
2. `src/backend/docker.rs` - Implement docker cp integration
3. `guest-agent/src/main.rs` - Extend guest agent with WriteFile/ReadFile
4. `src/vsock.rs` - Add new request types for file operations
