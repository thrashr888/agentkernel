
# Compliance Framework Mapping

Maps agentkernel enterprise controls to SOC 2, HIPAA, and FedRAMP.

## Overview

| Control Area | agentkernel Feature | SOC 2 | HIPAA | FedRAMP |
|---|---|---|---|---|
| Access Control | Cedar policies, JWT/OIDC, Security profiles | CC6.1 | 164.312(a)(1) | AC-3 |
| Audit Logging | Audit log, SIEM streaming | CC7.2 | 164.312(b) | AU-2 |
| Network Isolation | Per-VM controls, Domain filtering | CC6.6 | 164.312(e)(1) | SC-7 |
| Encryption | Ed25519 signing, TLS enforcement | CC6.7 | 164.312(e)(2) | SC-13 |
| Policy Management | Remote server, Signed bundles, Cache | CC6.1, CC6.6 | 164.312(a)(1) | AC-3, CM-3 |

---

## 1. Access Control

**Controls:**

- **Cedar policy engine** — deny-by-default policies governing what agents can do
- **JWT/OIDC authentication** — identity verification via Okta, Azure AD, Google Workspace
- **MFA enforcement** — Cedar policies can require `principal.mfa_verified == true`
- **Role-based access** — Cedar evaluates JWT claim roles to determine permitted actions
- **Security profiles** — permissive, moderate, restrictive with escalating controls
- **Unique user IDs** — JWT `sub` claim (HIPAA unique user identification)
- **Emergency access** — `offline_mode = "cached_with_expiry"` for provider outages (HIPAA)
- **Deny-by-default** — no action without explicit permit policy (FedRAMP)
- **Multi-tenant hierarchy** — Org > Team > User with forbid-always-wins (FedRAMP)

```toml
[enterprise]
enabled = true
policy_server = "https://policy.acme-corp.com"

[security]
profile = "restrictive"
```

```cedar
forbid(
    principal in AgentKernel::User,
    action == AgentKernel::Create,
    resource
) when { !principal.mfa_verified };

permit(
    principal in AgentKernel::User,
    action == AgentKernel::Run,
    resource
) when {
    principal.roles.contains("developer") &&
    resource.max_memory_mb <= 2048
};
```

---

## 2. Audit Logging

**Controls:**

- **Local audit log** — all operations logged to `~/.agentkernel/audit.jsonl`
- **SIEM streaming** — real-time events via HTTP webhooks
- **OCSF format** — Open Cybersecurity Schema Framework for SIEM compatibility
- **Policy decision logging** — every permit/deny with full context
- **File access logging** — `FileRead`/`FileWritten` events (HIPAA data access tracking)
- **Session tracking** — `SessionAttached` events for interactive sessions
- **Command logging** — every sandbox command logged with exit codes

```toml
[enterprise.audit_stream]
destination = { type = "http_webhook", url = "https://siem.acme-corp.com/ingest" }
batch_size = 50
flush_interval_secs = 30
ocsf_enabled = true
```

**Auditable events (FedRAMP AU-2):**

| Event | OCSF Class |
|---|---|
| `sandbox_created`, `sandbox_started`, `sandbox_stopped`, `sandbox_removed` | 3001 (API Activity) |
| `command_executed`, `file_written`, `file_read` | 3001 |
| `session_attached`, `policy_violation`, `policy_decision` | 3001 |
| `auth_event` | 3002 (Authentication) |

---

## 3. Network Isolation

**Controls:**

- **Per-VM network isolation** — each sandbox runs its own microVM with independent network stack
- **Network disable** — `network = false` removes all network access
- **Domain filtering** — allowlist/blocklist for reachable domains
- **Cloud metadata blocking** — default blocklist includes `169.254.169.254`
- **TLS enforcement** — all policy server communication over HTTPS (HIPAA transmission security)
- **Separate kernel** — each microVM has its own Linux kernel, preventing cross-sandbox sniffing
- **Hardware isolation via KVM** — each sandbox is a separate VM (FedRAMP boundary protection)
- **vsock communication** — host-guest uses vsock, not network, reducing attack surface

```toml
[security]
profile = "restrictive"
network = false

[security.domains]
allow = ["api.anthropic.com", "*.pypi.org"]
block = ["169.254.169.254", "metadata.google.internal"]
allowlist_only = true
```

---

## 4. Encryption

**Controls:**

- **Policy signing** — Ed25519 signatures on bundles (FIPS 186-5)
- **TLS transport** — all policy server communication over HTTPS
- **Token storage** — OIDC tokens at `~/.agentkernel/auth/tokens.json` with `0600` permissions
- **Trust anchors** — configurable public keys for policy verification
- **Version monotonicity** — prevents downgrade attacks
- **Read-only root** — `restrictive` profile limits data persistence (HIPAA)
- **Ephemeral sandboxes** — `agentkernel run` destroys after execution (HIPAA)

```toml
[enterprise.trust_anchors]
keys = ["ed25519-public-key-base64-1", "ed25519-public-key-base64-2"]
```

**Cryptographic controls (FedRAMP SC-13):**

| Component | Algorithm | Standard |
|---|---|---|
| Policy signing | Ed25519 | FIPS 186-5 |
| JWT validation | RS256/RS384/RS512 | RFC 7519 |
| TLS transport | TLS 1.2+ (rustls) | FIPS 140-2 compatible |

---

## 5. Policy Management

**Controls:**

- **Remote policy server** — centralized management for the organization
- **Pull-based model** — agents poll for updates, no inbound connections required
- **Signed bundles** — Ed25519 signatures verify authenticity and integrity
- **Version tracking** — monotonically increasing versions prevent rollback
- **Policy cache** — local cache at `~/.agentkernel/policies/` with expiry controls
- **Multi-tenant hierarchy** — Org > Team > User with forbid-always-wins
- **Trust anchor rotation** — multiple keys support rotation without downtime

**Offline modes:**

| Mode | Behavior | Security |
|---|---|---|
| `fail_closed` | Block all operations | Highest |
| `cached_with_expiry` | Use cached policies up to max_age | High (recommended) |
| `cached_indefinite` | Use cached policies forever | Medium |
| `default_policy` | Fall back to embedded defaults | Low |

---

## Evidence Collection

| Area | Evidence |
|---|---|
| Access Control | Cedar policy files, `policy_decision` audit entries, OIDC provider config, enterprise config |
| Audit | Audit log export, SIEM integration config, sample OCSF events, retention policy |
| Network | Security profile config, domain filtering rules, network enforcement audit entries |
| Encryption | Trust anchor config, signed policy bundles, TLS config, token storage permissions |
| Policy | Policy server config, bundle version history, offline mode config, tenant hierarchy |
