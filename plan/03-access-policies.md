# Plan: Complex Network and Command Access Policies

## Problem Statement

agentkernel currently provides three coarse-grained security profiles (permissive, moderate, restrictive) with binary network on/off control. This is insufficient for:

1. **Network Policies**: AI agents often need specific APIs (PyPI, npm, GitHub) while blocking everything else
2. **Command Execution**: No way to restrict which binaries are allowed
3. **No Syscall Filtering**: Docker backend has basic capability dropping but no seccomp profiles
4. **No Audit Trail**: Policy violations are silently blocked
5. **No Rate Limiting**: Sandboxes could exfiltrate unlimited data

## Extended Configuration Schema

```toml
[security]
profile = "custom"

[security.network]
enabled = true
default_action = "deny"

[[security.network.rules]]
action = "allow"
domains = ["pypi.org", "*.pypi.org", "github.com", "*.github.com"]
ports = [443]

[[security.network.rules]]
action = "allow"
domains = ["api.anthropic.com", "api.openai.com"]
ports = [443]

[security.network.limits]
bandwidth_mbps = 10
connections_per_minute = 100

[security.commands]
default_action = "allow"

[[security.commands.rules]]
action = "allow"
binaries = ["python", "python3", "pip", "pip3"]
paths = ["/usr/bin/*", "/usr/local/bin/*"]

[[security.commands.rules]]
action = "deny"
binaries = ["curl", "wget", "nc", "ncat", "ssh", "scp"]

[security.syscalls]
profile = "moderate"  # or path to JSON

[security.audit]
enabled = true
log_violations = true
```

## Module Structure

```
src/
├── policy/
│   ├── mod.rs          # Policy types and parsing
│   ├── network.rs      # NetworkPolicy struct
│   ├── commands.rs     # CommandPolicy struct
│   ├── syscalls.rs     # SeccompPolicy loading
│   ├── audit.rs        # Audit logging
│   └── engine.rs       # Policy evaluation engine
├── enforcement/
│   ├── mod.rs          # Enforcement abstraction
│   ├── docker.rs       # Docker-specific enforcement
│   ├── firecracker.rs  # Firecracker enforcement
│   └── dns_proxy.rs    # DNS filtering proxy
```

## Network Policy Enforcement

### Docker Backend
```rust
fn apply_network_policy(policy: &NetworkPolicy) -> Vec<String> {
    let mut args = Vec::new();
    if !policy.enabled {
        args.push("--network=none".to_string());
        return args;
    }
    args.push("--network=agentkernel-filtered".to_string());
    args.push(format!("--dns={}", DNS_PROXY_IP));
    args
}
```

### DNS Filtering Proxy

A lightweight DNS proxy that:
- Intercepts DNS queries from sandboxes
- Checks against domain allowlist/blocklist
- Returns NXDOMAIN for blocked domains
- Logs all queries for audit

```rust
pub struct DnsFilterProxy {
    allowed_domains: HashSet<String>,
    blocked_domains: HashSet<String>,
    default_action: PolicyAction,
    upstream: SocketAddr,
}
```

## Command Policy Enforcement

Enhance guest agent to check commands before execution:

```rust
async fn handle_run_request(request: AgentRequest, policy: &CommandPolicy) -> AgentResponse {
    let binary = &request.command[0];

    if !policy.is_allowed(binary) {
        return AgentResponse {
            exit_code: Some(-1),
            stderr: Some(format!("Command '{}' blocked by security policy", binary)),
            error: Some("POLICY_VIOLATION".to_string()),
            ..Default::default()
        };
    }

    execute_command(request.command).await
}
```

## Seccomp Profile Integration

Pre-built profiles:
```
images/seccomp/
├── permissive.json    # Minimal restrictions
├── moderate.json      # Block network creation, mount, reboot
├── restrictive.json   # Allowlist-only for typical agent operations
└── ai-agent.json      # Tailored for AI coding agents
```

Docker integration:
```rust
fn apply_seccomp(profile: &SeccompProfile) -> Vec<String> {
    vec![format!("--security-opt=seccomp={}", profile.path())]
}
```

## Audit Logging

```rust
#[derive(Debug, Serialize)]
pub struct PolicyViolation {
    pub timestamp: DateTime<Utc>,
    pub sandbox_name: String,
    pub violation_type: ViolationType,
    pub details: ViolationDetails,
    pub action_taken: ActionTaken,
}

pub enum ViolationType {
    NetworkDomainBlocked,
    NetworkPortBlocked,
    CommandBlocked,
    SyscallBlocked,
    RateLimitExceeded,
}
```

## Implementation Phases

### Phase 1: Foundation (1-2 days)
1. Create `src/policy/mod.rs` with core policy types
2. Extend `config.rs` to parse `[security.*]` sections
3. Add policy validation

### Phase 2: Network Domain Filtering (2-3 days)
1. Implement DNS filtering proxy
2. Integrate with Docker backend
3. Integrate with Firecracker (configure guest DNS)

### Phase 3: Command Filtering (1-2 days)
1. Add CommandPolicy to guest agent protocol
2. Modify guest agent to check commands
3. Implement binary path resolution

### Phase 4: Seccomp Integration (2-3 days)
1. Create pre-built seccomp profiles
2. Add `--security-opt seccomp=` to Docker backend
3. Enable CONFIG_SECCOMP in Firecracker kernel

### Phase 5: Audit Logging (1 day)
1. Implement AuditLogger with JSONL output
2. Add `agentkernel audit` CLI command

## Security Considerations

1. **Policy Bypass Prevention**: Block DNS-over-HTTPS, resolve symlinks in command filtering
2. **Defense in Depth**: VM boundary remains primary isolation; policies are additional layers
3. **Trusted Policy Source**: Policies come from host, never from inside sandbox
4. **Performance**: DNS proxy ~1-5ms latency, command filtering <1ms, seccomp near zero-cost

## Critical Files for Implementation

1. `src/permissions.rs` - Extend with fine-grained policies
2. `src/config.rs` - Add `[security.*]` sections
3. `src/backend/docker.rs` - Seccomp, network args
4. `guest-agent/src/main.rs` - Command filtering
5. `images/kernel/microvm.config` - Enable CONFIG_SECCOMP=y
