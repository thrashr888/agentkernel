# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability, please report it by submitting a GitHub issue.

**Please do NOT open a public issue for security vulnerabilities.**

We will respond within 48 hours and work with you to understand and address the issue.

---

## Security Model

### Isolation Architecture

agentkernel provides hardware-level isolation using **Firecracker microVMs**. Each sandbox runs in its own virtual machine with:

- **Dedicated Linux kernel**: Not shared with the host or other sandboxes
- **Hardware memory isolation**: Enforced via KVM/VT-x
- **Minimal attack surface**: Firecracker is ~50k lines of Rust
- **No container escapes**: Unlike Docker, there's no shared kernel to escape through

When Firecracker is unavailable (e.g., macOS without Docker), the system falls back to:
- **macOS Seatbelt** (sandbox-exec) for lightweight process isolation
- **Docker/Podman containers** with hardened security profiles

### Security Profiles

Three built-in security profiles control sandbox permissions:

| Profile | Network | Mount CWD | Mount Home | Pass Env | Privileged |
|---------|---------|-----------|------------|----------|------------|
| **Restrictive** | No | No | No | No | No |
| **Moderate** (default) | Yes | No | No | No | No |
| **Permissive** | Yes | Yes | Yes | Limited | No |

The default "Moderate" profile:
- Allows network access (required for package managers, APIs)
- Does NOT mount the current working directory
- Does NOT mount the home directory
- Does NOT pass environment variables
- Drops all container capabilities except essential ones (CHOWN, SETUID, SETGID)
- Enables `no-new-privileges` to prevent privilege escalation

### Input Validation

All user-provided inputs are validated before use:

| Input Type | Validation Rules |
|------------|-----------------|
| **Sandbox names** | Alphanumeric + hyphens/underscores only, 1-63 chars |
| **Runtime names** | Validated against allowlist (base, python, node, etc.) |
| **Docker images** | Must follow Docker naming conventions, no shell metacharacters |
| **File paths** | Absolute paths required, no parent directory references |

This prevents:
- Command injection via malicious names
- Path traversal attacks
- SBPL (Seatbelt Profile Language) injection on macOS

### Known Limitations

1. **Privileged Docker during setup**: Building rootfs images requires `--privileged` Docker access for loop device operations. This only runs during `agentkernel setup`, not during normal operation.

2. **HTTP API authentication**: The HTTP API server (`agentkernel serve`) does not implement authentication. Run it only on trusted networks or behind a reverse proxy with authentication.

3. **MCP server trust**: The MCP server accepts all JSON-RPC commands over stdio. Access is controlled by whoever can communicate with the process's stdio.

4. **Firecracker on macOS**: Requires Docker Desktop with nested virtualization, which adds latency and reduces isolation compared to native KVM on Linux.

---

## Hardening Recommendations

### Production Deployment

1. **Use Linux with native KVM** for maximum security and performance
2. **Bind HTTP API to localhost only** (`--host 127.0.0.1`)
3. **Use the Restrictive profile** when network access isn't needed
4. **Pre-build rootfs images** instead of building locally with privileged Docker
5. **Run agentkernel as a non-root user** (KVM group membership is sufficient)

### Container Runtime Hardening

When using Docker/Podman fallback:

```bash
# agentkernel automatically applies these security flags:
--cap-drop=ALL
--cap-add=CHOWN
--cap-add=SETUID
--cap-add=SETGID
--security-opt=no-new-privileges:true
--read-only  # (in Restrictive profile)
```

### Network Isolation

For maximum isolation, disable network access:

```bash
# CLI
agentkernel run --no-network python3 script.py

# Config file
[security]
network = false
```

---

## Security Audit History

### 2026-01-20: Initial Security Review

Conducted comprehensive security audit covering:
- Command injection vulnerabilities
- Path traversal attacks
- Sandbox escape vectors
- Privilege escalation risks
- Input validation gaps

**Issues Fixed:**
- Added strict validation for sandbox names (prevents shell injection)
- Added runtime allowlist (prevents path traversal)
- Added Seatbelt path validation (prevents SBPL injection)
- Fixed TOCTOU race conditions in socket cleanup
- Added read-only mount for build scripts
- Improved documentation of privileged Docker usage

---

## Threat Model

### In Scope

- Malicious code running inside sandboxes attempting to escape
- Malicious user input via CLI, HTTP API, or MCP server
- Privilege escalation from sandbox to host
- Data exfiltration from host filesystem

### Out of Scope

- Physical access to the host machine
- Compromised host operating system
- Side-channel attacks (Spectre, Meltdown)
- Denial of service via resource exhaustion (no resource limits implemented yet)
- Supply chain attacks on dependencies

---

## Compliance Notes

- **SOC 2**: Hardware isolation via microVMs supports strong logical separation
- **HIPAA**: Network isolation and encryption (TLS) capabilities support data protection
- **PCI-DSS**: Sandbox isolation can support segmentation requirements

Note: agentkernel alone does not guarantee compliance. Proper configuration and additional controls are required.
