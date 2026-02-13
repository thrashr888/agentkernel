
# Secrets

agentkernel provides secure secret management for AI agent sandboxes. Secrets can be stored in a local vault and delivered to sandboxes via two mechanisms: **network-layer proxy injection** (Gondolin pattern) and **file-based injection**.

## Quick Start

```bash
# Store a secret
agentkernel secret set OPENAI_API_KEY sk-proj-...

# Run a sandbox with proxy-injected secrets (Gondolin pattern)
agentkernel create my-agent --image python:3.12-slim \
  --secret OPENAI_API_KEY:api.openai.com

# Or inject secrets as files
agentkernel create my-agent --image python:3.12-slim \
  --secret-file OPENAI_API_KEY
```

## Secret Vault

The vault stores secrets locally with multiple backend options.

### Backends

| Backend | Description | Best For |
|---------|-------------|----------|
| `file` (default) | Encrypted file at `~/.agentkernel/secrets.json` with `0600` permissions | Local development |
| `env` | Read secrets from environment variables | CI/CD pipelines |
| `keyring` | OS keychain (macOS Keychain, Linux secret-service) | Production workstations |

### CLI Commands

```bash
# Store a secret (value in args)
agentkernel secret set ANTHROPIC_API_KEY sk-ant-...

# Store a secret (stdin, avoids shell history)
echo "sk-ant-..." | agentkernel secret set ANTHROPIC_API_KEY

# Retrieve
agentkernel secret get ANTHROPIC_API_KEY

# List all keys
agentkernel secret list

# Delete
agentkernel secret delete ANTHROPIC_API_KEY
```

### Storage Details

- **File backend**: `~/.agentkernel/secrets.json`, base64-encoded values, `0600` file permissions
- **Env backend**: Read-only — reads from process environment variables, cannot set/delete
- **Keyring backend**: Requires the `keyring` Cargo feature flag

## Injection Methods

### 1. Proxy Injection (Gondolin Pattern)

The recommended approach. Secrets never enter the sandbox VM — they are injected as HTTP headers by a host-side proxy.

**How it works:**

1. agentkernel starts an HTTP forward proxy on the host (one per sandbox)
2. `HTTP_PROXY` / `HTTPS_PROXY` env vars point the sandbox at the proxy
3. The proxy generates a CA certificate and injects it into the sandbox trust store
4. For HTTPS: the proxy terminates TLS (MITM), injects secret headers, then forwards upstream
5. For HTTP: the proxy injects headers directly
6. Unauthorized hosts receive a `403 Forbidden` response

```
┌─────────────────┐          ┌──────────────┐          ┌──────────────────┐
│    Sandbox VM   │──HTTP──▶ │  Host Proxy  │──HTTPS─▶ │  api.openai.com  │
│                 │          │              │          │                  │
│  curl https://  │          │ + Authorization:        │                  │
│  api.openai.com │          │   Bearer sk-proj-...    │                  │
│                 │          │              │          │                  │
│  (no secrets    │          │ (secrets     │          │                  │
│   in memory)    │          │  live here)  │          │                  │
└─────────────────┘          └──────────────┘          └──────────────────┘
```

**Usage:**

```bash
# Bind a vault secret to a host (defaults to Authorization: Bearer header)
agentkernel create my-agent --secret OPENAI_API_KEY:api.openai.com

# Inline value (useful for one-off testing)
agentkernel create my-agent --secret OPENAI_API_KEY=sk-proj-xxx:api.openai.com

# Custom header name (e.g., Anthropic uses x-api-key)
agentkernel create my-agent --secret ANTHROPIC_API_KEY:api.anthropic.com:x-api-key

# Multiple bindings
agentkernel create my-agent \
  --secret OPENAI_API_KEY:api.openai.com \
  --secret ANTHROPIC_API_KEY:api.anthropic.com:x-api-key \
  --secret GITHUB_TOKEN:api.github.com
```

**Binding syntax:**

| Format | Meaning |
|--------|---------|
| `KEY:host` | Look up KEY in vault, inject as `Authorization: Bearer <value>` to host |
| `KEY=value:host` | Use inline value, inject as `Authorization: Bearer <value>` to host |
| `KEY:host:header` | Look up KEY in vault, inject as `<header>: <value>` to host (no prefix) |

**What happens inside the sandbox:**

- Placeholder env vars are set (e.g., `OPENAI_API_KEY=ak-proxy-managed`) so tools don't fail existence checks
- The real secret value is never present in the VM's memory, environment, or filesystem
- `HTTP_PROXY`, `HTTPS_PROXY`, `http_proxy`, `https_proxy` are configured automatically
- The proxy CA cert is injected into the system trust store and language-specific CA paths (`NODE_EXTRA_CA_CERTS`, `REQUESTS_CA_BUNDLE`, `SSL_CERT_FILE`)

**Domain allowlisting:**

The proxy enforces that only bound hosts receive traffic. Requests to unauthorized hosts are blocked with `403 Forbidden`. This prevents secrets from leaking to unexpected destinations and stops code from exfiltrating data.

### 2. File-Based Injection

Secrets are written as files inside the sandbox at `/run/agentkernel/secrets/KEY`. Useful when code reads credentials from files rather than HTTP headers.

```bash
# Inject a vault secret as a file
agentkernel create my-agent --secret-file MY_SECRET

# Multiple files
agentkernel create my-agent \
  --secret-file DATABASE_URL \
  --secret-file SERVICE_ACCOUNT_JSON
```

**What happens inside the sandbox:**

- Each secret is written to `/run/agentkernel/secrets/<KEY>` with `0400` permissions (owner read-only)
- The env var `AGENTKERNEL_SECRETS_PATH=/run/agentkernel/secrets` is set
- Key names are validated: alphanumeric, underscores, and hyphens only

**Reading secrets from code:**

```python
import os
secrets_path = os.environ.get("AGENTKERNEL_SECRETS_PATH", "/run/agentkernel/secrets")
with open(f"{secrets_path}/DATABASE_URL") as f:
    db_url = f.read().strip()
```

### Combining Both Methods

You can use proxy injection and file injection together:

```bash
agentkernel create my-agent \
  --secret OPENAI_API_KEY:api.openai.com \
  --secret-file DATABASE_URL
```

## SDK Support

All five SDKs support both `secrets` (proxy bindings) and `secret_files` (file injection).

### TypeScript

```typescript
import { AgentKernel } from 'agentkernel';

const ak = new AgentKernel();
const sandbox = await ak.createSandbox('my-agent', {
  image: 'python:3.12-slim',
  secrets: ['OPENAI_API_KEY:api.openai.com'],
  secretFiles: ['DATABASE_URL'],
});
```

### Python

```python
from agentkernel import AgentKernel

ak = AgentKernel()
sandbox = ak.create_sandbox("my-agent",
    image="python:3.12-slim",
    secrets=["OPENAI_API_KEY:api.openai.com"],
    secret_files=["DATABASE_URL"],
)
```

### Go

```go
ak := agentkernel.New()
sandbox, err := ak.CreateSandbox(ctx, "my-agent", &agentkernel.CreateSandboxOptions{
    Image:       "python:3.12-slim",
    Secrets:     []string{"OPENAI_API_KEY:api.openai.com"},
    SecretFiles: []string{"DATABASE_URL"},
})
```

### Rust

```rust
let ak = AgentKernel::new()?;
let sandbox = ak.create_sandbox("my-agent", CreateSandboxOptions {
    image: Some("python:3.12-slim".into()),
    secrets: vec!["OPENAI_API_KEY:api.openai.com".into()],
    secret_files: vec!["DATABASE_URL".into()],
    ..Default::default()
}).await?;
```

### Swift

```swift
let ak = AgentKernel()
let sandbox = try await ak.createSandbox("my-agent", options: CreateSandboxOptions(
    image: "python:3.12-slim",
    secrets: ["OPENAI_API_KEY:api.openai.com"],
    secretFiles: ["DATABASE_URL"]
))
```

## HTTP API

### Create sandbox with secrets

```bash
curl -X POST http://localhost:18888/sandboxes \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-agent",
    "image": "python:3.12-slim",
    "secrets": ["OPENAI_API_KEY:api.openai.com"],
    "secret_files": ["DATABASE_URL"]
  }'
```

### Check proxy status

```bash
curl http://localhost:18888/sandboxes/my-agent/proxy
```

Returns proxy port, bound hosts, and CA fingerprint.

## Configuration

### agentkernel.toml

```toml
[secrets]
backend = "file"   # "file", "env", or "keyring"

# Pre-configured secret bindings (applied to all sandboxes)
OPENAI_API_KEY = { host = "api.openai.com" }
ANTHROPIC_API_KEY = { host = "api.anthropic.com", header = "x-api-key" }
```

## Security Model

### Threat model

| Threat | Proxy injection | File injection | Env var passthrough |
|--------|----------------|----------------|---------------------|
| Secret in VM memory | No | Yes (in file) | Yes |
| Visible in `ps` / `/proc` | No | No | Yes |
| Exfiltration to unauthorized host | Blocked by proxy | Possible | Possible |
| Secret survives snapshot | No | Depends on snapshot scope | Yes |

### Recommendations

1. **Use proxy injection** for API keys sent as HTTP headers. This is the strongest option — secrets never enter the VM.
2. **Use file injection** for credentials that tools read from disk (database URLs, service account JSON files).
3. **Avoid env var passthrough** (`-e KEY=value`) when possible. Environment variables are visible to all processes and appear in `/proc/*/environ`.
4. **Use the vault** instead of inline values. Inline values (`KEY=value:host`) appear in shell history and process listings on the host.
5. **Pipe secrets via stdin** when setting vault values: `echo "value" | agentkernel secret set KEY`.

## Proxy Hooks

Register webhooks to monitor proxied requests in real time.

```bash
# Register a webhook for all proxied requests
curl -X POST http://localhost:18888/proxy/hooks \
  -H "Content-Type: application/json" \
  -d '{
    "name": "audit-logger",
    "event": "on_request",
    "target": { "type": "webhook", "url": "http://localhost:9999/audit" }
  }'

# List registered hooks
curl http://localhost:18888/proxy/hooks

# Remove a hook
curl -X DELETE http://localhost:18888/proxy/hooks/audit-logger
```

Hook payloads include: timestamp, sandbox name, method, URL, host, status code, latency, and whether a secret was injected.

## Desktop App

The agentkernel desktop app includes a **Secrets** page for managing vault contents through a GUI. You can add, view, and delete secrets without using the CLI.

## See Also

- [Secret CLI Commands](cmd-secrets) — vault management commands
- [Security Profiles](config-security) — domain filtering, command filtering, seccomp
- [SDK Reference](sdks) — full SDK documentation
- [Getting Started](getting-started) — first sandbox walkthrough
