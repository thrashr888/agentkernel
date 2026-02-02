# Plan: Secure Transport Layer (SSH + TLS)

## Problem Statement

agentkernel provides `exec` for running commands inside sandboxes, but this requires control-plane API access and doesn't integrate with standard developer tooling (VS Code Remote SSH, `scp`, `rsync`, CI pipelines). The HTTP API runs plain HTTP. Port-mapped services expose unencrypted traffic.

For HashiCorp (Customer Zero) and enterprise customers, all sandbox access should be:
- **Encrypted in transit** — no plain HTTP, no unencrypted TCP by default
- **Identity-bound** — access tied to a user/agent identity, not just network reachability
- **Time-bounded** — credentials expire automatically
- **Auditable** — every connection logged with identity, action, duration

SSH with Vault-signed certificates solves the access problem. TLS with ACME/Vault PKI solves the transport problem. Together they give us: "every connection is encrypted, every connection is authenticated, every credential expires."

## Why SSH

- Every developer already has an SSH client
- Every IDE supports SSH remoting (VS Code, JetBrains, Cursor)
- Every CI system speaks SSH
- SSH with certificates is **better gated** than `exec` — requires both Vault access AND a signed cert
- Well-audited protocol with decades of hardening
- No custom protocol, no proprietary client

## Design

### 1. SSH Access with Vault-Signed Certificates

**Flow: `agentkernel ssh <sandbox>`**

```
Developer                     agentkernel CLI            Vault                    Sandbox
    |                              |                       |                        |
    |-- agentkernel ssh web ------>|                       |                        |
    |                              |-- generate ephemeral  |                        |
    |                              |   keypair (ed25519)   |                        |
    |                              |                       |                        |
    |                              |-- POST /ssh/sign/     |                        |
    |                              |   agentkernel-client   |                        |
    |                              |   {public_key, ttl}   |                        |
    |                              |<-- signed certificate--|                        |
    |                              |   (5-30 min TTL)      |                        |
    |                              |                       |                        |
    |                              |-- ssh -i cert         |                        |
    |                              |   user@sandbox:port ---------------------->    |
    |                              |                       |                        |
    |<-- interactive shell --------|                       |                        |
```

**Vault SSH Secrets Engine setup:**
- Role `agentkernel-client` with allowed users, TTL, extensions
- CA public key distributed to all sandbox sshd configs
- No static keys anywhere — every credential is ephemeral

**Sandbox-side:**
- sshd configured with `TrustedUserCAKeys /etc/ssh/vault-ca.pub`
- Password auth disabled
- Only certificate auth accepted
- CA pubkey injected at sandbox creation time

### 2. VS Code / IDE Integration

```bash
# Generate SSH config entry for a sandbox
agentkernel ssh-config web
# Output:
# Host agentkernel-web
#   HostName localhost
#   Port 2222
#   User sandbox
#   ProxyCommand agentkernel ssh-proxy web
#   StrictHostKeyChecking no
#   UserKnownHostsFile /dev/null

# Add all sandboxes to SSH config
agentkernel ssh-config --all >> ~/.ssh/config

# VS Code: Remote-SSH → Connect to Host → agentkernel-web
```

**ProxyCommand approach:**
- `agentkernel ssh-proxy <sandbox>` handles Vault signing transparently
- Signs a new cert on each connection (short TTL)
- Streams stdin/stdout to the sandbox SSH port
- Works with any SSH client, any IDE

### 3. HTTPS for the HTTP API

**Current:** `http_api.rs` serves plain HTTP on a local socket.

**Proposed:**
- Add `--tls` flag to `agentkernel serve` (HTTP API server)
- When `--tls` is set, use `rustls` for TLS termination
- Certificate sources (priority order):
  1. `--tls-cert` / `--tls-key` — user-provided cert/key files
  2. Vault PKI — request cert from Vault PKI secrets engine
  3. ACME — Let's Encrypt (for public-facing deployments)
  4. Self-signed — auto-generated for local dev (with warning)
- Add `--require-tls` to reject plain HTTP connections entirely

### 4. TLS for Port-Mapped Services

When sandboxes expose ports, the traffic between host and sandbox is unencrypted. Options:

**Option A: Sidecar proxy (recommended)**
- Inject an Envoy/HAProxy sidecar that terminates TLS
- Cert provisioned from Vault PKI or ACME
- Transparent to the application inside the sandbox
- Works with any protocol (HTTP, gRPC, WebSocket)

**Option B: Application-level TLS**
- Inject cert/key files into the sandbox
- Application configures TLS itself
- Less overhead, more burden on the user

**Recommendation:** Option A for HTTP services, Option B as escape hatch.

### 5. Transport Security Policy

New config section and Cedar policy dimension:

```toml
[security.transport]
require_encrypted = true    # Block plain HTTP/TCP port mappings
ssh = true                  # Enable SSH access (installs sshd)
vault_addr = ""             # Vault address (from VAULT_ADDR if empty)
vault_ssh_mount = "ssh"     # Vault SSH secrets engine mount
vault_ssh_role = "agentkernel-client"
cert_ttl = "30m"            # Default certificate TTL
```

**Cedar policies for transport:**
```cedar
// Require TLS for all port mappings in production
forbid(
    principal,
    action == AgentKernel::Action::"PortMap",
    resource
) when {
    resource.require_encrypted == true &&
    context.protocol != "tls" && context.protocol != "ssh"
};

// Allow SSH only for specific roles
permit(
    principal in AgentKernel::Group::"platform-team",
    action == AgentKernel::Action::"SSH",
    resource
);
```

### 6. New CLI Commands

```bash
# SSH into a sandbox
agentkernel ssh <sandbox>
agentkernel ssh <sandbox> -- ls -la        # run single command

# Generate SSH config
agentkernel ssh-config <sandbox>           # single sandbox
agentkernel ssh-config --all               # all running sandboxes

# SSH proxy (used by ProxyCommand)
agentkernel ssh-proxy <sandbox>

# Create sandbox with SSH enabled
agentkernel create web --ssh -p 8080:80
agentkernel run --ssh -P 2222:22 python3 server.py

# Start API server with TLS
agentkernel serve --tls --tls-cert cert.pem --tls-key key.pem
agentkernel serve --tls --vault-pki                    # cert from Vault PKI
agentkernel serve --tls --acme --domain api.example.com  # Let's Encrypt
```

## Implementation Order

### Phase 1: SSH Core
1. **sshd injection** — `--ssh` flag on `create`/`run`, injects sshd + config into sandbox
2. **Vault SSH integration** — sign ephemeral keys via Vault SSH secrets engine
3. **`agentkernel ssh` command** — generates cert, connects
4. **`agentkernel ssh-config`** — generates SSH config for IDE integration
5. **`agentkernel ssh-proxy`** — ProxyCommand for transparent cert signing

### Phase 2: TLS for API
6. **rustls integration** — `--tls` flag for `agentkernel serve`
7. **Vault PKI integration** — request API server cert from Vault
8. **ACME support** — Let's Encrypt for public deployments
9. **`--require-tls` enforcement** — reject plain HTTP

### Phase 3: Transport Policy
10. **Transport security config** — `[security.transport]` section
11. **Cedar SSH action** — `SSH` action in enterprise policy
12. **Encrypted-only port mappings** — block unencrypted exposed services

### Phase 4: Advanced
13. **TLS sidecar for port-mapped services** — auto-inject TLS termination
14. **SSH session recording** — audit trail via asciicast (already have the format)
15. **SSH key agent forwarding** — allow agents to use developer's SSH keys (for git)

## Dependencies

- `rustls` + `rustls-pemfile` — TLS implementation (no OpenSSL dependency)
- `russh` or `thrussh` — SSH protocol (optional, for ssh-proxy without shelling out)
- Vault HTTP API — no Vault SDK needed, just REST calls
- `rcgen` — self-signed cert generation for local dev
- `acme-lib` or `instant-acme` — ACME/Let's Encrypt client

## Security Considerations

- **No password auth** — certificate-only SSH, always
- **Short TTLs** — 5-30 min certs, re-signed on each connection
- **No static keys** — ephemeral keypairs generated per session
- **CA trust** — sandbox only trusts the Vault CA, nothing else
- **Audit logging** — every SSH connection logged with identity + duration
- **Key agent forwarding opt-in** — disabled by default, explicit flag to enable
- **Restrictive profile** — `--ssh` is incompatible with `restrictive` profile (no network)

## Open Questions

1. Should `--ssh` be implied when using `--profile permissive`?
2. Do we want a built-in SSH CA for non-Vault users, or require Vault?
3. Should the TLS sidecar use Envoy, HAProxy, or a custom Rust proxy?
4. ACME rate limits — how do we handle many short-lived sandboxes needing certs?
5. Should SSH session recordings be stored locally or shipped to a central audit log?
