# Firecracker Daemon Mode Architecture

## Overview

Design a daemon mode for agentkernel that maintains a pool of pre-warmed Firecracker VMs for fast execution. The CLI will use the daemon if running, falling back to ephemeral mode if not.

## User Requirements

- CLI can use daemon if running, falls back to ephemeral mode if not
- Pool size: 3-5 pre-warmed VMs
- Firecracker backend only (not Docker)

## Key Design Decisions

### 1. VsockClient is Stateless
The VsockClient can reconnect to existing VMs at any time - it doesn't hold VM state. This means:
- Daemon owns and manages VM lifecycles
- CLI connects to daemon via Unix socket
- Daemon hands out vsock paths to pre-warmed VMs

### 2. Detach VMs from Drop
Current problem: `FirecrackerVm::Drop` kills the VM process. For daemon mode:
- Daemon-owned VMs use `mem::forget()` or a flag to skip Drop cleanup
- Ephemeral VMs still get cleaned up normally

### 3. Pool Architecture
Based on the existing ContainerPool pattern:
- Warm pool (VecDeque of ready VMs)
- Semaphore for concurrent VM starts
- Background health checking and replenishment

## Data Structures

```rust
// src/daemon/pool.rs
pub struct PooledVm {
    pub id: String,
    pub cid: u32,
    pub vsock_path: PathBuf,
    pub created_at: Instant,
    pub last_used: Instant,
    pub runtime: String,
}

pub struct FirecrackerPool {
    warm_pool: Mutex<VecDeque<PooledVm>>,
    in_use: Mutex<HashMap<String, PooledVm>>,
    config: PoolConfig,
    start_semaphore: Semaphore,
}

pub struct PoolConfig {
    pub min_warm: usize,      // 3
    pub max_warm: usize,      // 5
    pub max_age: Duration,    // 5 minutes
    pub health_interval: Duration,
}
```

```rust
// src/daemon/state.rs
pub struct DaemonState {
    pub pool: Arc<FirecrackerPool>,
    pub socket_path: PathBuf,
}
```

## Implementation Phases

### Phase 1: Core Pool Structure
**Files:** `src/daemon/mod.rs`, `src/daemon/pool.rs`

1. Create `FirecrackerPool` with warm pool management
2. Implement `acquire()` - get VM from pool or start new one
3. Implement `release()` - return VM to pool or destroy if stale
4. Add background task for pool replenishment

### Phase 2: Daemon Server
**Files:** `src/daemon/server.rs`, `src/daemon/protocol.rs`

1. Unix socket server at `~/.agentkernel/daemon.sock`
2. Simple JSON protocol:
   - `{"cmd": "acquire", "runtime": "python"}` → `{"id": "...", "vsock": "..."}`
   - `{"cmd": "release", "id": "..."}` → `{"ok": true}`
   - `{"cmd": "status"}` → `{"warm": 3, "in_use": 1}`
3. Handle client disconnects (release VMs back to pool)

### Phase 3: CLI Integration
**Files:** `src/main.rs`, `src/vmm.rs`

1. Add daemon client in CLI
2. Detection: check if `~/.agentkernel/daemon.sock` exists and is responsive
3. Flow for `agentkernel run`:
   ```
   if daemon_available():
       vm = daemon.acquire(runtime)
       result = exec_via_vsock(vm.vsock_path, command)
       daemon.release(vm.id)
   else:
       # Existing ephemeral flow
       vm = FirecrackerVm::new()
       vm.start()
       result = vm.exec(command)
       vm.stop()
   ```

### Phase 4: Health Checking
**Files:** `src/daemon/health.rs`

1. Periodic ping to all warm VMs
2. Remove unresponsive VMs from pool
3. Auto-replenish to maintain min_warm
4. Track VM age and recycle stale VMs

### Phase 5: CLI Commands
**Files:** `src/main.rs`

1. `agentkernel daemon start` - start daemon in background
2. `agentkernel daemon stop` - graceful shutdown
3. `agentkernel daemon status` - show pool stats
4. Consider `--foreground` flag for debugging

## Files to Create

```
src/
├── daemon/
│   ├── mod.rs          # Module exports
│   ├── pool.rs         # FirecrackerPool implementation
│   ├── server.rs       # Unix socket server
│   ├── protocol.rs     # JSON protocol types
│   ├── health.rs       # Health checking
│   └── client.rs       # Client for CLI to connect
```

## Files to Modify

- `src/main.rs` - Add daemon subcommand and CLI integration
- `src/vmm.rs` - Add `detach()` method to prevent Drop cleanup
- `src/lib.rs` - Export daemon module

## Testing Strategy

1. Unit tests for pool logic (acquire/release/replenish)
2. Integration test: start daemon, run commands, verify reuse
3. Stress test: many concurrent requests
4. Failure tests: VM crash recovery, daemon restart

## Verification

1. Build: `cargo build`
2. Start daemon: `agentkernel daemon start`
3. Check status: `agentkernel daemon status` (should show 3 warm VMs)
4. Run command: `agentkernel run -- echo hello` (should use warm VM, <50ms)
5. Run again: verify reuse via daemon status
6. Stop daemon: `agentkernel daemon stop`
7. Run without daemon: `agentkernel run -- echo hello` (should fall back to ephemeral)

## Performance Targets

| Metric | Current (Ephemeral) | Target (Daemon) |
|--------|---------------------|-----------------|
| First command | ~500ms | ~500ms (cold start) |
| Subsequent commands | ~500ms | <50ms (warm pool) |
| VM reuse rate | 0% | >90% |

## Risks and Mitigations

1. **VM leak on daemon crash**: Store VM PIDs in file, cleanup on startup
2. **Resource exhaustion**: Hard cap on total VMs (warm + in_use)
3. **Stale VMs**: Age-based recycling + health checks
