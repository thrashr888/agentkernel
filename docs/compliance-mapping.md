# Compliance Framework Mapping

This document maps agentkernel enterprise controls to common compliance frameworks: SOC 2, HIPAA, and FedRAMP. For each control, we describe the agentkernel feature that satisfies it, how to configure it, and what evidence to collect for auditors.

## Overview Matrix

| Control Area | agentkernel Feature | SOC 2 | HIPAA | FedRAMP |
|---|---|---|---|---|
| Access Control | Cedar policies, JWT/OIDC auth, Security profiles | CC6.1 | 164.312(a)(1) | AC-3 |
| Audit Logging | Audit log, Stream to SIEM | CC7.2 | 164.312(b) | AU-2 |
| Network Isolation | Per-VM network controls, Domain filtering | CC6.6 | 164.312(e)(1) | SC-7 |
| Encryption | Ed25519 policy signing, TLS enforcement | CC6.7 | 164.312(e)(2) | SC-13 |
| Policy Management | Remote policy server, Signed bundles, Cache | CC6.1, CC6.6 | 164.312(a)(1) | AC-3, CM-3 |

---

## 1. Access Control

### SOC 2 -- CC6.1: Logical and Physical Access Controls

**Requirement**: The entity implements logical access security measures to protect against unauthorized access.

**agentkernel Controls**:
- **Cedar policy engine**: Fine-grained, deny-by-default access policies that govern what agents can do
- **JWT/OIDC authentication**: Identity verification via enterprise identity providers (Okta, Azure AD, Google Workspace)
- **MFA enforcement**: Cedar policies can require `principal.mfa_verified == true` for sensitive operations
- **Role-based access**: Cedar policies evaluate roles from JWT claims to determine permitted actions
- **Security profiles**: Three built-in profiles (permissive, moderate, restrictive) with escalating controls

**Configuration**:
```toml
[enterprise]
enabled = true
policy_server = "https://policy.acme-corp.com"

[security]
profile = "restrictive"
```

```cedar
# Require MFA for sandbox creation
forbid(
    principal in AgentKernel::User,
    action == AgentKernel::Create,
    resource
) when {
    !principal.mfa_verified
};

# Allow developers to run sandboxes with limited resources
permit(
    principal in AgentKernel::User,
    action == AgentKernel::Run,
    resource
) when {
    principal.roles.contains("developer") &&
    resource.max_memory_mb <= 2048
};
```

**Evidence Collection**:
- Export Cedar policy files showing access rules
- Audit log entries for `policy_decision` events (permit/deny)
- JWT validation logs showing identity verification
- OIDC provider configuration showing MFA requirements

### HIPAA -- 164.312(a)(1): Access Control

**Requirement**: Implement technical policies and procedures for electronic information systems that maintain electronic protected health information (ePHI) to allow access only to authorized persons.

**agentkernel Controls**:
- Same as SOC 2 CC6.1 controls above
- **Unique user identification**: JWT `sub` claim provides unique user IDs
- **Emergency access**: `offline_mode = "cached_with_expiry"` allows access during provider outages with time-limited cached policies
- **Automatic logoff**: Sandbox sessions have configurable timeouts
- **Encryption**: All policy communication over TLS; policy bundles signed with Ed25519

**Configuration**:
```toml
[enterprise]
enabled = true
offline_mode = "cached_with_expiry"
cache_max_age_hours = 4  # Short cache for healthcare

[security]
profile = "restrictive"
network = false  # No network for ePHI processing sandboxes
```

### FedRAMP -- AC-3: Access Enforcement

**Requirement**: The information system enforces approved authorizations for logical access to information and system resources.

**agentkernel Controls**:
- **Cedar policy evaluation**: All sandbox operations require explicit permit decisions
- **Deny-by-default**: No action is allowed without an explicit permit policy
- **Policy signing**: Ed25519 signatures prevent unauthorized policy modifications
- **Multi-tenant hierarchy**: Org > Team > User scoping with forbid-always-wins semantics
- **Audit trail**: Every policy decision is logged with full context

---

## 2. Audit Logging

### SOC 2 -- CC7.2: Monitoring of System Components

**Requirement**: The entity monitors system components and the operation of those components for anomalies.

**agentkernel Controls**:
- **Local audit log**: All sandbox operations logged to `~/.agentkernel/audit.jsonl`
- **Audit streaming**: Real-time event streaming to external SIEM systems via HTTP webhooks
- **OCSF format**: Events follow the Open Cybersecurity Schema Framework for SIEM compatibility
- **Policy decision logging**: Every permit/deny decision includes full context (who, what, when, why)
- **Tamper evidence**: Log entries include timestamps, process IDs, and user context

**Configuration**:
```toml
[enterprise.audit_stream]
destination = { type = "http_webhook", url = "https://siem.acme-corp.com/ingest" }
batch_size = 50
flush_interval_secs = 30
ocsf_enabled = true
```

**Evidence Collection**:
- Export audit log entries filtered by date range
- SIEM dashboard showing agentkernel events
- Stream configuration showing real-time monitoring is active
- Sample OCSF events demonstrating compliance metadata

### HIPAA -- 164.312(b): Audit Controls

**Requirement**: Implement hardware, software, and/or procedural mechanisms that record and examine activity in information systems that contain or use ePHI.

**agentkernel Controls**:
- Same as SOC 2 CC7.2 controls above
- **File access logging**: `FileRead` and `FileWritten` audit events track data access
- **Session tracking**: `SessionAttached` events log interactive sessions
- **Command execution logging**: Every command executed in a sandbox is logged with exit codes

### FedRAMP -- AU-2: Audit Events

**Requirement**: The organization determines that the information system is capable of auditing specific events.

**agentkernel Auditable Events**:

| Event Type | Description | OCSF Class |
|---|---|---|
| `sandbox_created` | New sandbox instantiated | 3001 (API Activity) |
| `sandbox_started` | Sandbox started with security profile | 3001 |
| `sandbox_stopped` | Sandbox stopped | 3001 |
| `sandbox_removed` | Sandbox destroyed | 3001 |
| `command_executed` | Command run inside sandbox | 3001 |
| `file_written` | File written to sandbox | 3001 |
| `file_read` | File read from sandbox | 3001 |
| `session_attached` | Interactive session started | 3001 |
| `policy_violation` | Policy denied an action | 3001 |
| `policy_decision` | Policy evaluation result | 3001 |
| `auth_event` | Authentication attempt | 3002 (Authentication) |

---

## 3. Network Isolation

### SOC 2 -- CC6.6: Restrictions on Logical Access

**Requirement**: The entity restricts logical access to the system based on implemented access controls.

**agentkernel Controls**:
- **Per-VM network isolation**: Each sandbox runs in its own microVM with independent network stack
- **Network disable**: `network = false` completely removes network access from a sandbox
- **Domain filtering**: Allowlist/blocklist control over which domains sandboxes can reach
- **Cloud metadata blocking**: Default blocklist includes cloud metadata endpoints (169.254.169.254)
- **Agent-specific policies**: Each AI agent type has tailored network policies

**Configuration**:
```toml
[security]
profile = "restrictive"
network = false  # Complete network isolation

[security.domains]
allow = ["api.anthropic.com", "*.pypi.org"]
block = ["169.254.169.254", "metadata.google.internal"]
allowlist_only = true
```

### HIPAA -- 164.312(e)(1): Transmission Security

**Requirement**: Implement technical security measures to guard against unauthorized access to ePHI transmitted over electronic communications networks.

**agentkernel Controls**:
- **Network isolation**: Sandboxes processing ePHI can run with `network = false`
- **TLS enforcement**: All communication with policy server uses HTTPS with certificate verification
- **Domain allowlisting**: Only explicitly approved endpoints can be reached
- **Separate kernel**: Each microVM has its own Linux kernel, preventing cross-sandbox sniffing

### FedRAMP -- SC-7: Boundary Protection

**Requirement**: The information system monitors and controls communications at external boundaries and key internal boundaries.

**agentkernel Controls**:
- **Hardware isolation via KVM**: Each sandbox is a separate virtual machine
- **No shared kernel**: Unlike containers, sandboxes cannot escape to host via kernel exploits
- **vsock communication**: Host-guest communication uses vsock (not network), reducing attack surface
- **Firewall per VM**: Each microVM can have independent network policies

---

## 4. Encryption

### SOC 2 -- CC6.7: Encryption in Transit and at Rest

**Requirement**: The entity uses encryption to protect data in transit and at rest.

**agentkernel Controls**:
- **Policy signing**: Ed25519 signatures on policy bundles (FIPS 186-5 approved)
- **TLS for policy transport**: All policy server communication over HTTPS
- **Token storage**: OIDC tokens stored with 0600 permissions at `~/.agentkernel/auth/tokens.json`
- **Trust anchors**: Configurable public key trust anchors for policy verification
- **Version monotonicity**: Policy version checks prevent downgrade attacks

**Configuration**:
```toml
[enterprise.trust_anchors]
keys = ["ed25519-public-key-base64-1", "ed25519-public-key-base64-2"]
```

### HIPAA -- 164.312(e)(2): Encryption

**Requirement**: Implement a mechanism to encrypt electronic protected health information whenever deemed appropriate.

**agentkernel Controls**:
- Same as SOC 2 CC6.7 controls above
- **Read-only root filesystem**: `restrictive` profile mounts root as read-only, limiting data persistence
- **Ephemeral sandboxes**: `agentkernel run` creates temporary sandboxes that are destroyed after execution

### FedRAMP -- SC-13: Cryptographic Protection

**Requirement**: The information system implements required cryptographic protections using cryptographic modules that comply with applicable federal laws, directives, policies, and regulations.

**agentkernel Cryptographic Controls**:

| Component | Algorithm | Standard |
|---|---|---|
| Policy signing | Ed25519 | FIPS 186-5 |
| JWT validation | RS256/RS384/RS512 | RFC 7519 |
| TLS transport | TLS 1.2+ with rustls | FIPS 140-2 compatible |
| Key derivation | N/A (delegated to IdP) | Per IdP certification |

---

## 5. Policy Management

### SOC 2 -- CC6.1 + CC6.6: Change Management and Access Control

**agentkernel Controls**:
- **Remote policy server**: Centralized policy management for the entire organization
- **Pull-based model**: Agents poll for policy updates (no inbound connections required)
- **Signed policy bundles**: Ed25519 signatures verify policy authenticity and integrity
- **Policy versioning**: Monotonically increasing versions prevent rollback attacks
- **Offline modes**: Configurable behavior when policy server is unreachable
- **Policy cache**: Local cache at `~/.agentkernel/policies/` with expiry controls
- **Multi-tenant hierarchy**: Org > Team > User with forbid-always-wins resolution

**Offline Mode Options**:

| Mode | Behavior | Security Level |
|---|---|---|
| `fail_closed` | Block all sandbox operations | Highest |
| `cached_with_expiry` | Use cached policies up to max_age | High (recommended) |
| `cached_indefinite` | Use cached policies forever | Medium |
| `default_policy` | Fall back to embedded defaults | Low |

### FedRAMP -- CM-3: Configuration Change Control

**agentkernel Controls**:
- **Policy audit trail**: All policy changes are logged
- **Signed bundles**: Only authorized administrators can create valid policy bundles
- **Version tracking**: Each policy bundle has a version number for change tracking
- **Trust anchor rotation**: Multiple trust anchors support key rotation without downtime

---

## Evidence Collection Guide

For each compliance audit, collect the following evidence from agentkernel:

### Access Control Evidence
1. Cedar policy files defining access rules
2. OIDC provider configuration (showing SSO/MFA setup)
3. Audit log filtered for `policy_decision` events
4. Enterprise config showing policy server integration

### Audit Evidence
1. Audit log export (`~/.agentkernel/audit.jsonl`)
2. SIEM integration configuration
3. Sample OCSF events from audit stream
4. Log retention policy documentation

### Network Isolation Evidence
1. Security profile configuration (`agentkernel.toml`)
2. Domain filtering rules
3. Audit log showing network policy enforcement
4. Architecture diagram showing per-VM isolation

### Encryption Evidence
1. Trust anchor configuration
2. Policy bundle with Ed25519 signature
3. TLS configuration for policy server communication
4. Token storage permissions verification

### Policy Management Evidence
1. Policy server configuration
2. Policy bundle version history
3. Offline mode configuration
4. Multi-tenant hierarchy definition
