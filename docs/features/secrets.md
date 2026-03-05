
# Secrets

agentkernel stores secrets in a local vault and delivers them to sandboxes via three methods, ranked by security:

1. **Proxy injection** — secrets stay on host, injected as HTTP headers by a MITM proxy
2. **Placeholder tokens** — random tokens in files, real values substituted by proxy in outbound traffic
3. **File injection** — real values written to files inside the sandbox

## Quick Start

```bash
# Store a secret
agentkernel secret set OPENAI_API_KEY sk-proj-...

# Proxy injection (recommended — secret never enters VM)
agentkernel sandbox create my-agent \
  --secret OPENAI_API_KEY:api.openai.com

# File injection with placeholder tokens (secret never enters VM)
agentkernel sandbox create my-agent \
  --secret-file OPENAI_API_KEY --placeholder-secrets

# File injection (real value on disk)
agentkernel sandbox create my-agent --secret-file OPENAI_API_KEY
```

## Vault

### Backends

| Backend | Storage | Writable | Best For |
|---------|---------|----------|----------|
| `file` (default) | `~/.agentkernel/secrets.json` (`0600`) | Yes | Local dev |
| `env` | Host environment variables | No | CI/CD |
| `keyring` | OS keychain (macOS Keychain, Linux secret-service) | Yes | Production |

The `keyring` backend requires the `keyring` Cargo feature.

### CLI

```bash
agentkernel secret set KEY value       # store (or pipe: echo "val" | agentkernel secret set KEY)
agentkernel secret get KEY             # retrieve
agentkernel secret list                # list keys
agentkernel secret delete KEY          # delete
```

## Injection Methods

### 1. Proxy Injection

Secrets never enter the sandbox. A host-side proxy injects them as HTTP headers on outbound requests.

```
┌─────────────────┐          ┌──────────────┐          ┌──────────────────┐
│    Sandbox VM   │──HTTP──▶ │  Host Proxy  │──HTTPS─▶ │  api.openai.com  │
│                 │          │              │          │                  │
│  (no secrets    │          │ + Authorization:        │                  │
│   in memory)    │          │   Bearer sk-proj-...    │                  │
└─────────────────┘          └──────────────┘          └──────────────────┘
```

**Binding syntax:**

| Format | Meaning |
|--------|---------|
| `KEY:host` | Vault lookup, inject as `Authorization: Bearer <value>` |
| `KEY=value:host` | Inline value, inject as `Authorization: Bearer <value>` |
| `KEY:host:header` | Vault lookup, inject as custom header (no Bearer prefix) |

```bash
agentkernel sandbox create my-agent \
  --secret OPENAI_API_KEY:api.openai.com \
  --secret ANTHROPIC_API_KEY:api.anthropic.com:x-api-key \
  --secret GITHUB_TOKEN:api.github.com
```

Inside the sandbox: placeholder env vars are set (e.g., `OPENAI_API_KEY=ak-proxy-managed`) so tools pass existence checks. Proxy env vars and CA certs are configured automatically. Only bound hosts receive traffic — unauthorized hosts get `403 Forbidden`.

### 2. File Injection

Secrets written to `/run/agentkernel/secrets/<KEY>` with `0400` permissions.

```bash
agentkernel sandbox create my-agent \
  --secret-file DATABASE_URL \
  --secret-file SERVICE_ACCOUNT_JSON
```

Reading from code:

```python
import os
secrets_path = os.environ.get("AGENTKERNEL_SECRETS_PATH", "/run/agentkernel/secrets")
with open(f"{secrets_path}/DATABASE_URL") as f:
    db_url = f.read().strip()
```

### 3. Placeholder Tokens

Like file injection, but files contain random tokens (`AGENTKERNEL_PLACEHOLDER_<hex>`) instead of real values. The proxy substitutes real values in outbound HTTP traffic. Real secrets never enter the VM.

```bash
agentkernel sandbox create my-agent \
  --secret-file OPENAI_API_KEY \
  --placeholder-secrets
```

Use this when code needs to read credentials from files but you want host-only secret storage. Exfiltrated tokens are meaningless without the proxy.

### Combining Methods

```bash
agentkernel sandbox create my-agent \
  --secret OPENAI_API_KEY:api.openai.com \
  --secret-file DATABASE_URL \
  --placeholder-secrets
```

`--placeholder-secrets` applies to all `--secret-file` entries. Proxy injection always keeps secrets on the host regardless.

## SDKs

All SDKs accept `secrets`, `secret_files`, and `placeholder_secrets`:

```typescript
// TypeScript
const sandbox = await ak.createSandbox('my-agent', {
  secrets: ['OPENAI_API_KEY:api.openai.com'],
  secretFiles: ['DATABASE_URL'],
  placeholderSecrets: true,
});
```

```python
# Python
sandbox = ak.create_sandbox("my-agent",
    secrets=["OPENAI_API_KEY:api.openai.com"],
    secret_files=["DATABASE_URL"],
    placeholder_secrets=True,
)
```

See [SDK Reference](../sdks/index.md) for Go, Rust, and Swift examples.

## HTTP API

```bash
curl -X POST http://localhost:18888/sandboxes \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-agent",
    "secrets": ["OPENAI_API_KEY:api.openai.com"],
    "secret_files": ["DATABASE_URL"],
    "placeholder_secrets": true
  }'

# Check proxy status
curl http://localhost:18888/sandboxes/my-agent/proxy
```

## Configuration

```toml
[secrets]
backend = "file"   # "file", "env", or "keyring"

# Pre-configured bindings (applied to all sandboxes)
OPENAI_API_KEY = { host = "api.openai.com" }
ANTHROPIC_API_KEY = { host = "api.anthropic.com", header = "x-api-key" }
```

## Security Model

| Threat | Proxy | Placeholder tokens | File | Env var |
|--------|-------|--------------------|------|---------|
| Secret in VM memory | No | No | Yes | Yes |
| Visible in `/proc` | No | No | No | Yes |
| Exfiltration to unauthorized host | Blocked | Blocked (tokens meaningless) | Possible | Possible |
| Survives snapshot | No | No (tokens only) | Yes | Yes |

**Recommendations:**

1. Use `--secret KEY:host` for HTTP API keys. Secrets never enter the VM.
2. Use `--secret-file KEY --placeholder-secrets` when code reads from files. Tokens are substituted by the proxy.
3. Use `--secret-file KEY` only for credentials that must be real on disk (TLS certs, SSH keys).
4. Avoid `-e KEY=value` — env vars are visible to all processes via `/proc`.
5. Pipe secrets via stdin to avoid shell history: `echo "val" | agentkernel secret set KEY`

## Proxy Hooks

Monitor proxied requests with webhooks:

```bash
curl -X POST http://localhost:18888/proxy/hooks \
  -H "Content-Type: application/json" \
  -d '{
    "name": "audit-logger",
    "event": "on_request",
    "target": { "type": "webhook", "url": "http://localhost:9999/audit" }
  }'
```

Payloads include timestamp, sandbox name, method, URL, host, status code, latency, and whether a secret was injected.

## See Also

- [Secret CLI Commands](../commands/secrets.md) — vault management
- [Security Profiles](../config/security.md) — domain filtering, command filtering, seccomp
- [SDK Reference](../sdks/index.md) — Go, Rust, Swift examples
- [Desktop App](../app/index.md) — GUI for managing vault secrets
- [Getting Started](../getting-started/quick-start.md) — first sandbox walkthrough
