# Plan: Enterprise Remote Policy Management

## Problem Statement

Currently, agentkernel policies are configured entirely locally. For enterprise deployments, this is insufficient:
- No centralized control over what agents can do across an organization
- No way to enforce compliance policies remotely
- No audit trail of policy decisions
- No cryptographic verification that policies haven't been tampered with
- No support for organizational hierarchies or tenant isolation

## Architecture: Pull-Based with Cedar Policies

```
Enterprise Policy Server (AWS/GCP)
        |
        v
    [HTTPS REST API]
        |
        v
    Agent (polls every N seconds)
        |
        v
    Local Policy Cache (~/.agentkernel/policies/)
        |
        v
    Sandbox Enforcement
```

**Why Cedar for Policies?**
| Aspect | Cedar | OPA/Rego |
|--------|-------|----------|
| Performance | 42-60x faster | Slower |
| Readability | High | Medium (Prolog-like) |
| Safety | Deterministic, deny-by-default | Can have runtime exceptions |

## Policy Schema Design

```cedar
namespace AgentKernel {
    entity User {
        email: String,
        org_id: String,
        roles: Set<String>,
        mfa_verified: Boolean,
    };

    entity Sandbox {
        name: String,
        agent_type: String,
        runtime: String,
    };

    action Run, Exec, Create, Attach, Mount, Network;
}

// Example policies
permit(
    principal in AgentKernel::User,
    action == AgentKernel::Run,
    resource == AgentKernel::Permission
) when {
    principal.roles.contains("developer") &&
    resource.max_memory_mb <= 2048
};

forbid(
    principal,
    action == AgentKernel::Network,
    resource
) when {
    principal.org_id == "healthcare-corp" &&
    !principal.mfa_verified
};
```

## Cryptographic Policy Signing

**Algorithm**: Ed25519 (FIPS 186-5 approved)

```rust
pub struct PolicyBundle {
    pub policies: Vec<CedarPolicy>,
    pub version: u64,
    pub expires_at: Option<DateTime<Utc>>,
    pub signature: [u8; 64],    // Ed25519 signature
    pub signer_key_id: String,  // Key identifier for rotation
}

pub struct TrustAnchor {
    pub key_id: String,
    pub public_key: [u8; 32],  // Ed25519 public key
    pub valid_from: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
}
```

**Verification Flow:**
1. Fetch policy bundle from server (HTTPS)
2. Verify Ed25519 signature against embedded trust anchors
3. Check signature timestamp against policy expiry
4. If valid, update local cache and enforce

## Offline Mode

```rust
pub enum OfflineMode {
    /// Fail all sandbox operations if server unreachable
    FailClosed,
    /// Use cached policy, refuse after cache expiry
    CachedWithExpiry { max_age: Duration },
    /// Use cached policy indefinitely (least secure)
    CachedIndefinite,
    /// Fall back to embedded default policy
    DefaultPolicy,
}
```

**Recommended Default**: `CachedWithExpiry { max_age: Duration::from_secs(86400) }` (24 hours)

## Configuration

```toml
[enterprise]
enabled = true
policy_server = "https://policy.acme-corp.com"
org_id = "acme-corp"
api_key_env = "AGENTKERNEL_API_KEY"
offline_mode = "cached_with_expiry"
cache_max_age_hours = 24

[enterprise.trust_anchors]
keys = ["age1xxxxxxxxxx", "age1yyyyyyyyyy"]
```

## Multi-Tenancy Architecture

```
Enterprise Policy Server
    +-- Org: acme-corp
    |       +-- Team: platform
    |       +-- Team: ml-research
    +-- Org: globex-inc
            +-- Team: engineering
```

**Resolution Order**: User > Team > Org > Global (most specific wins, `forbid` always wins over `permit`)

## Identity Integration

**Phase 1 (API Key + JWT):**
```rust
pub struct AgentIdentity {
    pub api_key: Option<String>,
    pub jwt_claims: Option<JwtClaims>,
}

pub struct JwtClaims {
    pub sub: String,
    pub email: String,
    pub org_id: String,
    pub roles: Vec<String>,
    pub mfa_verified: bool,
}
```

**Phase 2**: OIDC/SAML with Okta, Azure AD, Google Workspace, Auth0

## Compliance Framework Mapping

| Control | SOC 2 | HIPAA | FedRAMP |
|---------|-------|-------|---------|
| Access control | CC6.1 | 164.312(a)(1) | AC-3 |
| Audit logging | CC7.2 | 164.312(b) | AU-2 |
| Network isolation | CC6.6 | 164.312(e)(1) | SC-7 |
| Encryption | CC6.7 | 164.312(e)(2) | SC-13 |

## Implementation Phases

### Phase 1: Foundation (2-3 weeks)
**New Files:**
- `src/policy/mod.rs` - Module root
- `src/policy/client.rs` - HTTP client for policy server
- `src/policy/cache.rs` - Local policy cache
- `src/policy/signing.rs` - Ed25519 verification
- `src/policy/cedar.rs` - Cedar policy evaluation
- `src/policy/audit.rs` - Decision logging

**Dependencies:** `cedar-policy`, `ed25519-dalek`, `reqwest`

### Phase 2: Identity and Audit (2-3 weeks)
- JWT token validation with JWKS
- OIDC device flow for CLI authentication
- Audit log streaming to external systems

### Phase 3: Advanced Features (3-4 weeks)
- gRPC streaming for real-time policy updates
- Multi-tenant management UI
- Automatic rollback on anomaly detection

## Security Considerations

1. **Trust Anchor Compromise**: Key rotation, multi-key signing, HSM storage
2. **Cache Poisoning**: Verify signatures on every load
3. **Downgrade Attack**: Version monotonicity check
4. **Man-in-the-Middle**: TLS certificate verification
5. **Offline Mode Abuse**: Strict cache expiry, heartbeat requirements

## Critical Files for Implementation

1. `src/permissions.rs` - Add `from_cedar_policy()` method
2. `src/vmm.rs` - Integrate policy evaluation before sandbox creation
3. `src/daemon/server.rs` - Pattern for Unix socket communication
4. `src/config.rs` - Add `[enterprise]` section
5. `src/validation.rs` - Apply to policy content validation
