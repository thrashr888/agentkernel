# Plan: Agent Integration Workflow

## Executive Summary

This plan analyzes how AI coding agents (Claude Code, Gemini CLI, OpenAI Codex) use sandboxed execution and outlines how agentkernel can become a drop-in replacement or enhancement.

## Problem Statement

AI coding agents need to execute arbitrary code safely. Current solutions have trade-offs:

| Challenge | Impact |
|-----------|--------|
| Security vs Speed | Stronger isolation (VMs) means slower startup |
| Platform Fragmentation | Each agent implements its own sandbox |
| No Standard Protocol | MCP exists but sandbox tools lack standardization |
| Permission Complexity | Users must configure policies per-agent |

**agentkernel's opportunity**: Provide a unified, fast, secure sandbox backend.

## How Agents Handle Sandboxing Today

| Agent | Linux | macOS | Isolation | Network Control |
|-------|-------|-------|-----------|-----------------|
| Claude Code | bubblewrap | Seatbelt | Filesystem + Network proxy | Domain allowlist |
| Gemini CLI | Docker/Podman | sandbox-exec | Project directory | Docker flags |
| Codex CLI | Landlock + seccomp | Seatbelt | Filesystem + Network | Disabled by default |

**Key Insights:**
- Claude Code uses a proxy server outside the sandbox for network isolation
- Codex combines Landlock (filesystem) + seccomp (syscall filtering) on Linux
- Gemini CLI uses Docker by default

## What Agents Expect from a Sandbox

**P0 - Must Have:**
- Command execution with exit code
- stdout/stderr capture (batch or streaming)
- Working directory control
- Timeout enforcement
- Filesystem isolation

**P1 - High Priority:**
- Network control (on/off, domain allowlist)
- Environment variable passthrough
- File read/write within sandbox
- Multiple commands in same session

**P2 - Nice to Have:**
- Interactive shell/PTY support
- Resource limits
- Snapshot/restore
- File upload/download API
- Streaming output

## Enhanced MCP Tools

```json
{
  "name": "sandbox_run",
  "inputSchema": {
    "properties": {
      "command": {"type": "array"},
      "cwd": {"type": "string"},
      "env": {"type": "object"},
      "timeout_ms": {"type": "integer", "default": 30000},
      "profile": {"type": "string", "enum": ["restrictive", "moderate", "permissive"]},
      "network": {
        "properties": {
          "enabled": {"type": "boolean"},
          "allowed_domains": {"type": "array"}
        }
      },
      "streaming": {"type": "boolean", "default": false}
    },
    "required": ["command"]
  }
}
```

**New Tools Needed:**
- `sandbox_session_create` - Persistent session for multi-command workflows
- `sandbox_file_write` - Write files into sandbox
- `sandbox_file_read` - Read files from sandbox

## HTTP API Extensions

Add streaming support:
```
POST /run/stream
Content-Type: application/json

Response: Server-Sent Events
event: stdout
data: {"text": "Running tests..."}

event: exit
data: {"code": 0, "duration_ms": 1234}
```

## Compatibility Modes

```rust
pub enum CompatibilityMode {
    Native,      // Default agentkernel behavior
    ClaudeCode,  // Claude-compatible (proxy-style network)
    Codex,       // Codex-compatible (Landlock-style)
    Gemini,      // Gemini-compatible (Docker-style)
}
```

## Implementation Phases

### Phase 1: MCP Enhancement (1-2 weeks)
1. Add streaming output support
2. Add file read/write tools
3. Add session-based sandbox tool
4. Add domain-based network allowlisting

### Phase 2: Agent-Specific Adapters (2-3 weeks)
1. Claude Code adapter
2. Codex adapter
3. Gemini adapter
4. Per-agent configuration profiles

### Phase 3: Performance Optimization (1-2 weeks)
1. Warm pool pre-configuration per agent type
2. Speculative pre-warming based on usage patterns
3. Connection pooling for MCP sessions

**Targets:**
| Scenario | Current | Target |
|----------|---------|--------|
| First command (cold) | 800ms | 500ms |
| Subsequent commands (warm) | 195ms | 100ms |
| File write + exec | 400ms | 200ms |

### Phase 4: Claude Code Native Integration (2-3 weeks)
1. Create Claude Code plugin
2. Implement network proxy compatibility
3. Add approval flow integration
4. Write migration guide

### Phase 5: HTTP API as Universal Backend (1-2 weeks)
1. Add OpenAPI spec
2. Add authentication (API keys, JWT)
3. Generate SDKs (TypeScript, Python)

## Go-to-Market

**Message:** "The fastest, most secure local sandbox for AI coding agents"

**Key Differentiators:**
1. **Hardware isolation** - Real microVMs, not just containers
2. **Local-first** - Your code never leaves your machine
3. **Universal** - Works with Claude, Codex, Gemini
4. **Fast** - 195ms warm, competitive with native sandboxes

**Target Users:**
- Developers wanting stronger security than built-in sandboxes
- Enterprise teams needing audit trails and custom policies

## Competitor Analysis

| Feature | agentkernel | E2B | Cloudflare Sandbox |
|---------|-------------|-----|-------------------|
| Model | Local | Cloud SaaS | Cloud Edge |
| Isolation | Firecracker | Firecracker | V8 Isolates |
| Latency | ~195ms | ~seconds | ~ms |
| Language | Any | Any | JavaScript only |
| Self-hosted | Yes | No | No |

## Critical Files for Implementation

1. `src/mcp.rs` - Core MCP server; needs streaming, new tools
2. `src/backend/mod.rs` - Sandbox trait; needs agent-specific permission models
3. `src/permissions.rs` - Needs network allowlist, compatibility modes
4. `src/http_api.rs` - Needs streaming support, authentication
5. `claude-plugin/skills/sandbox/SKILL.md` - Critical for Claude Code adoption
