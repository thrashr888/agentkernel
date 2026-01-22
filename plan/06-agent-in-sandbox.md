# Plan: Running AI Agents INSIDE the Sandbox with Full TTY Support

## Problem Statement

Currently, agentkernel runs AI coding agents (Claude Code, Gemini CLI, Codex, OpenCode) on the host, sandboxing only the commands they generate. This creates a security gap: the agent itself has full host access.

The vision is to run the AI agent entirely within the sandbox, giving users:
1. **True agent isolation**: Agent and all tool executions in same isolated environment
2. **Full TTY/terminal support**: Interactive terminal experience
3. **Controlled external access**: Policy-based network and file access
4. **Multi-environment support**: Local machines AND hosted cloud environments
5. **Audit trail**: Complete session recording

## Current State

**TTY/Attach Support (src/main.rs):**
- `attach` command exists but is NOT implemented
- No PTY allocation or terminal handling code

**vsock Communication (src/vsock.rs):**
- Supports `Run`, `Shell`, `Ping`, `Shutdown` types
- `Shell` type defined but NOT implemented in guest agent
- Uses JSON-RPC over length-prefixed messages

**Guest Agent (guest-agent/src/main.rs):**
- Handles `Run`, `Ping`, `Shutdown` only
- NO PTY support - uses `Stdio::null()` for stdin
- NO interactive shell capability

## Architecture: Local-First with vsock PTY Bridge

```
┌────────────────────────────────────────────────────────────┐
│                          Host                               │
│  ┌──────────────┐     ┌───────────────┐     ┌────────────┐ │
│  │ Host PTY     │────▶│ vsock Bridge  │────▶│ Network    │ │
│  │ Multiplexer  │     │               │     │ Proxy      │ │
│  └──────────────┘     └───────────────┘     └────────────┘ │
│         │                     │                     │       │
│   ══════│═════════════════════│═════════════════════│══════ │
│         │                     │                     │       │
│  ┌──────▼─────────────────────▼─────────────────────▼─────┐ │
│  │              Firecracker microVM                        │ │
│  │  ┌─────────────────────────────────────────────────┐   │ │
│  │  │  Guest Agent (enhanced)                          │   │ │
│  │  │  ├── PTY Allocator                               │   │ │
│  │  │  ├── Session Manager                             │   │ │
│  │  │  └── Proxy Client                                │   │ │
│  │  └──────────────────────┬──────────────────────────┘   │ │
│  │                         │                               │ │
│  │              ┌──────────▼──────────┐                    │ │
│  │              │   claude / gemini   │                    │ │
│  │              │   (AI Agent)        │                    │ │
│  │              └─────────────────────┘                    │ │
│  └─────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

## Guest Agent PTY Allocator

```rust
// guest-agent/src/pty.rs
pub struct PtySession {
    master_fd: i32,
    slave_fd: i32,
    child_pid: Option<Pid>,
}

impl PtySession {
    pub fn spawn(command: &str, args: &[String]) -> Result<Self> {
        let OpenptyResult { master, slave } = openpty(None, None)?;

        match unsafe { fork()? } {
            ForkResult::Child => {
                setsid()?;
                unsafe { libc::ioctl(slave, libc::TIOCSCTTY, 0) };
                dup2(slave, 0)?; dup2(slave, 1)?; dup2(slave, 2)?;
                close(master)?; close(slave)?;
                exec::execvp(command, args)?;
            }
            ForkResult::Parent { child } => {
                close(slave)?;
                Ok(Self { master_fd: master, slave_fd: slave, child_pid: Some(child) })
            }
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let winsize = libc::winsize { ws_row: rows, ws_col: cols, .. };
        unsafe { libc::ioctl(self.master_fd, libc::TIOCSWINSZ, &winsize) };
        Ok(())
    }
}
```

## vsock Shell Protocol

```rust
pub enum RequestType {
    Run, Ping, Shutdown,
    Shell { command: String, args: Vec<String>, rows: u16, cols: u16 },
    ShellInput { session_id: String, data: Vec<u8> },
    ShellResize { session_id: String, rows: u16, cols: u16 },
    ShellClose { session_id: String },
}

pub enum ResponseType {
    ShellStarted { id: String, session_id: String },
    ShellOutput { session_id: String, data: Vec<u8> },
    ShellExited { session_id: String, exit_code: i32 },
}
```

## Network Proxy Architecture

All network access goes through a policy-controlled proxy:

```toml
[network]
mode = "proxy"

[network.proxy]
always_allow = [
    "api.anthropic.com",
    "api.openai.com",
    "generativelanguage.googleapis.com",
]
allow = [
    "*.pypi.org",
    "*.npmjs.com",
    "*.github.com",
]
block = [
    "169.254.169.254",  # Cloud metadata
    "*.internal",
]

[network.proxy.limits]
requests_per_minute = 60
bytes_per_hour = "100MB"
```

## Session Recording (asciicast v2 format)

```rust
pub struct SessionRecorder {
    output: BufWriter<File>,
    start_time: Instant,
}

impl SessionRecorder {
    pub fn record_output(&mut self, data: &[u8]) -> Result<()> {
        let event = AsciicastEvent {
            time: self.start_time.elapsed().as_secs_f64(),
            event_type: "o".to_string(),
            data: String::from_utf8_lossy(data).to_string(),
        };
        serde_json::to_writer(&mut self.output, &event)?;
        Ok(())
    }
}
```

**Playback:** `agentkernel replay <recording-id>`

## Implementation Phases

### Phase 1: PTY Foundation (2-3 weeks)
1. Guest agent PTY support via `openpty()`
2. Session management for multiple PTYs
3. Host terminal bridge with raw mode handling
4. Integration with `agentkernel attach` command

### Phase 2: Agent Launch in Sandbox (1-2 weeks)
1. Add agent binaries to rootfs images
2. Secure API key injection via vsock
3. New CLI: `agentkernel agent start <sandbox> --agent claude`

### Phase 3: Controlled Access Proxies (2-3 weeks)
1. SOCKS5 proxy server on host
2. Policy engine with allow/block lists
3. File system proxy with policy-based access
4. DNS proxy for controlled resolution

### Phase 4: Session Recording (1 week)
1. asciicast recording format
2. Storage in ~/.agentkernel/recordings/
3. Compression with zstd

### Phase 5: Hosted Environment Support (3-4 weeks)
1. WebSocket PTY API (`/sandboxes/{id}/attach`)
2. Multi-user support with namespacing
3. Authentication (API keys, OAuth)
4. Kubernetes deployment (Helm chart)

## Security Considerations

| Threat | Mitigation |
|--------|------------|
| Agent escapes sandbox | Firecracker/KVM hardware isolation |
| Agent exfiltrates data | Network proxy with allowlist |
| Agent reads sensitive files | File proxy with policy |
| Agent DoS host | Resource limits (cgroups) |
| Recording tampering | Append-only logs, checksums |

**API Key Security:** Never pass as command-line arguments (visible in ps); inject via vsock after sandbox starts.

## CLI Commands

```bash
# Start sandboxed Claude session
agentkernel agent start my-project --agent claude --project .
agentkernel agent attach my-project

# Replay session
agentkernel replay 20260122-143022
```

## Comparison with Existing Solutions

| Feature | Codespaces | Gitpod | Replit | agentkernel |
|---------|------------|--------|--------|-------------|
| Isolation | Container | Container | Container | VM (Firecracker) |
| TTY Support | Yes (SSH) | Yes (SSH) | Yes | Yes (vsock PTY) |
| Network Policy | Limited | Limited | Limited | Full proxy control |
| File Policy | N/A | N/A | N/A | Per-file policy |
| Session Recording | No | No | No | Yes (asciicast) |
| AI Agent Focus | No | No | Partial | Primary focus |
| Self-hostable | Enterprise | Yes | No | Yes |

## Critical Files for Implementation

1. `guest-agent/src/main.rs` - Add PTY support (Shell handlers)
2. `src/vsock.rs` - Implement Shell protocol
3. `src/main.rs` - Implement `attach` command (lines 337-357)
4. `src/backend/mod.rs` - Add `attach()` method to Sandbox trait
5. `src/http_api.rs` - Add WebSocket endpoint for hosted PTY
